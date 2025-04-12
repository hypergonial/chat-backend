use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{
        State,
        ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::any,
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use secrecy::ExposeSecret;
use serde::Serialize;
use tokio::{
    sync::{Mutex, broadcast, mpsc},
    time::timeout,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use crate::{
    app::App,
    models::{
        auth::Token,
        errors::GatewayError,
        gateway_event::{GatewayEvent, GatewayMessage, GuildCreatePayload},
        snowflake::Snowflake,
        user::{Presence, User},
    },
    utils::join_handle::JoinHandleExt,
};

use super::actor::{ConnectionId, GatewayCloseCode, GatewayRequest, GatewayResponse, SendMode, SessionHandle};

/// Default heartbeat interval in milliseconds
const HEARTBEAT_INTERVAL: u64 = 45000;

/// Get router for handling the gateway
///
/// ## Returns
///
/// A filter that can be used to handle the gateway
pub fn get_router() -> Router<App> {
    Router::new().route("/", any(websocket_handler))
}

async fn websocket_handler(State(app): State<App>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| async move { handle_connection(app, socket).await })
}

/// Send a serializable object to the client
///
/// ## Panics
///
/// This function will panic if the serializable object cannot be serialized
///
/// ## Arguments
///
/// * `ws_sink` - The sink for sending messages to the client
/// * `ser` - The serializable object to send
///
/// ## Returns
///
/// `Ok(())` if the message was sent successfully, an error otherwise
async fn send_serializable(
    ws_sink: &mut SplitSink<WebSocket, Message>,
    ser: impl Serialize,
) -> Result<(), axum::Error> {
    let message = serde_json::to_string(&ser).expect("Expected Serializable object to not fail serialization");
    ws_sink.send(Message::Text(message.into())).await
}

/// Send a close frame to the client
///
/// ## Arguments
///
/// * `ws_sink` - The sink for sending messages to the client
///
/// This fails silently if the close frame could not be sent, logging a warning
async fn send_close_frame(
    ws_sink: &mut SplitSink<WebSocket, Message>,
    code: GatewayCloseCode,
    reason: impl Into<Utf8Bytes>,
) {
    if let Err(e) = ws_sink
        .send(Message::Close(Some(CloseFrame {
            code: code.into(),
            reason: reason.into(),
        })))
        .await
    {
        tracing::debug!(error = %e, "Failed to send close frame");
    }
}

/// Send HELLO, then wait for and validate the IDENTIFY payload
///
/// ## Arguments
///
/// * `ws_sink` - The sink for sending messages to the client
/// * `ws_stream` - The stream for receiving messages from the client
///
/// ## Returns
///
/// The resolved user if the handshake was successful
async fn handle_handshake(
    app: App,
    ws_sink: &mut SplitSink<WebSocket, Message>,
    ws_stream: &mut SplitStream<WebSocket>,
) -> Result<User, GatewayError> {
    // Send HELLO with the heartbeat interval
    ws_sink
        .send(Message::Text(
            serde_json::to_string(&GatewayEvent::Hello {
                heartbeat_interval: HEARTBEAT_INTERVAL,
            })
            .expect("Failed to serialize HELLO payload")
            .into(),
        ))
        .await
        .ok();

    // IDENTIFY should be the first message sent
    let Ok(Some(Ok(ident))) = timeout(Duration::from_secs(5), ws_stream.next()).await else {
        send_close_frame(ws_sink, GatewayCloseCode::PolicyViolation, "IDENTIFY expected").await;
        return Err(GatewayError::HandshakeFailure("IDENTIFY expected".into()));
    };

    let Message::Text(text) = ident else {
        send_close_frame(ws_sink, GatewayCloseCode::Unsupported, "Unsupported message encoding").await;
        return Err(GatewayError::MalformedFrame("Unsupported message encoding".into()));
    };

    let Ok(GatewayMessage::Identify { token }) = serde_json::from_str(&text) else {
        send_close_frame(ws_sink, GatewayCloseCode::InvalidPayload, "Invalid IDENTIFY payload").await;
        return Err(GatewayError::MalformedFrame("Invalid IDENTIFY payload".into()));
    };

    let Ok(token) = Token::validate(app.clone(), token.expose_secret()).await else {
        send_close_frame(ws_sink, GatewayCloseCode::PolicyViolation, "Invalid token").await;
        return Err(GatewayError::AuthError("Invalid token".into()));
    };

    let Some(user) = app.ops().fetch_user(token.data().user_id()).await else {
        send_close_frame(ws_sink, GatewayCloseCode::ServerError, "No user belongs to token").await;
        return Err(GatewayError::InternalServerError("No user belongs to token".into()));
    };

    Ok(user)
}

/// Handle the heartbeat mechanism for a given user
///
/// This function will only return if the user failed to send a valid heartbeat within the timeframe
///
/// ## Arguments
///
/// * `app` - The shared application state
/// * `heartbeat_interval` - The interval at which heartbeats should be received from the user
/// * `user_id` - The ID of the user to receive heartbeats from
async fn handle_heartbeating(
    sender: Arc<broadcast::Sender<GatewayMessage>>,
    app: App,
    id: ConnectionId,
    heartbeat_interval: Duration,
) {
    loop {
        let mut recv = sender.subscribe();
        let sleep_task = tokio::time::sleep(heartbeat_interval + Duration::from_secs(5));
        // Wait for a single heartbeat message
        let heartbeat_task = tokio::spawn(async move {
            loop {
                let msg = recv.recv().await.map_err(|_| GatewayCloseCode::InvalidPayload)?;
                if matches!(msg, GatewayMessage::Heartbeat) {
                    return Ok(()); // Heartbeat received, we're good
                }
            }
        })
        .abort_on_drop();

        // Close if either the time runs out or an invalid payload is received
        let should_close = tokio::select! {
            () = sleep_task => Err(GatewayCloseCode::PolicyViolation),
            ret = heartbeat_task => ret.unwrap_or(Err(GatewayCloseCode::ServerError)),
        };

        if let Err(close_code) = should_close {
            let reason = match close_code {
                GatewayCloseCode::InvalidPayload => "Invalid payload".into(),
                GatewayCloseCode::PolicyViolation => "No HEARTBEAT received within timeframe".into(),
                _ => "Unknown error".into(),
            };

            app.gateway().close_session(id, close_code, reason);
            // We must not break here as that causes a race condition where the heartbeat
            // task may abort the sender task before the close message is sent
            // The sender task will stop when seeing the close anyway and
            // consequently abort all other WS tasks (including this one)
        } else {
            app.gateway().send_to_session(id, GatewayEvent::HeartbeatAck);
        }
    }
}

/// Send the `READY` event, all `GUILD_CREATE` events, and dispatch a `PRESENCE_UPDATE` event for this user
///
/// ## Arguments
///
/// * `app` - The shared application state
/// * `user` - The user to send the `READY` event to
/// * `ws_sink` - The sink for sending messages to the user
async fn send_onboarding_payloads(
    app: App,
    user: User,
    ws_sink: Arc<Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<(), axum::Error> {
    let guilds = app
        .ops()
        .fetch_guilds_for(&user)
        .await
        .expect("Failed to fetch guilds during socket connection handling");

    let read_states = app
        .ops()
        .fetch_read_states(user.id())
        .await
        .expect("Failed to fetch read states during socket connection handling");

    // Send READY
    send_serializable(
        &mut *ws_sink.lock().await,
        GatewayEvent::Ready {
            user: user.clone(),
            guilds: guilds.clone(),
            read_states,
        },
    )
    .await?;

    // Send GUILD_CREATE events for all guilds the user is in
    for guild in guilds {
        let payload = GuildCreatePayload::from_guild(&app, guild)
            .await
            .expect("Failed to fetch guild payload data");

        send_serializable(&mut *ws_sink.lock().await, GatewayEvent::GuildCreate(payload)).await?;
    }

    // Send the presence update for the user if they were not invisible when last logging off
    match user.last_presence() {
        Presence::Offline => {}
        _ => {
            app.gateway().dispatch(
                GatewayEvent::PresenceUpdate {
                    user_id: user.id(),
                    presence: *user.last_presence(),
                },
                SendMode::ToMutualGuilds(user.id()),
            );
        }
    }
    Ok(())
}

/// Forward events received through the `ConnectionHandle` receiver to the user
///
/// ## Arguments
///
/// * `user_id` - The ID of the user to send events to
/// * `receiver` - The receiver for incoming gateway responses to send
/// * `ws_sink` - The sink for sending messages to the user
async fn send_events(
    user_id: Snowflake<User>,
    mut receiver: UnboundedReceiverStream<GatewayResponse>,
    ws_sink: Arc<Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<GatewayCloseCode, axum::Error> {
    while let Some(payload) = receiver.next().await {
        match payload {
            GatewayResponse::Close(code, reason) => {
                tracing::debug!(?code, ?reason, "Closing connection for user {user_id}");
                send_close_frame(&mut *ws_sink.lock().await, code, reason).await;
                return Ok(code);
            }
            GatewayResponse::Event(event) => {
                let res = send_serializable(&mut *ws_sink.lock().await, event).await;
                if let Err(e) = res {
                    tracing::warn!(error = %e, "Error sending event to user {user_id}: {e}");
                    return Err(e);
                }
            }
        }
    }
    Ok(GatewayCloseCode::Normal)
}

/// Parse & forward events received through the socket to the `ConnectionHandle` sender
///
/// ## Arguments
///
/// * `user_id` - The ID of the user to receive events for
/// * `ws_stream` - The stream for receiving messages from the user
/// * `ws_sink` - The sink for sending messages to the user
async fn receive_events(
    conn_id: ConnectionId,
    mut ws_stream: SplitStream<WebSocket>,
    ws_sink: Arc<Mutex<SplitSink<WebSocket, Message>>>,
    broadcaster: Arc<broadcast::Sender<GatewayMessage>>,
) {
    while let Some(msg) = ws_stream.next().await {
        // Close if the user sends a close frame
        if let Ok(Message::Close(f)) = msg {
            tracing::debug!(close_frame = ?f, "Gateway stream closed by {conn_id}: {f:?}");
            break;
        }
        // Otherwise attempt to parse the message and send it
        let Ok(Message::Text(text)) = msg else {
            send_close_frame(
                &mut *ws_sink.lock().await,
                GatewayCloseCode::Unsupported,
                "Unsupported message encoding",
            )
            .await;
            break;
        };

        match serde_json::from_str::<GatewayRequest>(&text) {
            Ok(GatewayRequest::Message(msg)) => {
                tracing::debug!(?msg, "Received message from {conn_id}");
                if let Err(e) = broadcaster.send(msg) {
                    tracing::error!(error = %e, "Failed to broadcast message, all receivers dropped");
                    send_close_frame(
                        &mut *ws_sink.lock().await,
                        GatewayCloseCode::ServerError,
                        "Internal Server Error",
                    )
                    .await;
                    break;
                }
            }
            Err(e) => {
                send_close_frame(
                    &mut *ws_sink.lock().await,
                    GatewayCloseCode::InvalidPayload,
                    format!("Invalid request payload: {e}"),
                )
                .await;
                break;
            }
        }
    }
}

/// Handle a new websocket connection
///
/// ## Arguments
///
/// * `app` - The shared application state
/// * `socket` - The websocket connection to handle
async fn handle_connection(app: App, socket: WebSocket) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    if !app.gateway().is_started() {
        send_close_frame(&mut ws_sink, GatewayCloseCode::ServiceRestart, "Gateway is restarting").await;
        return;
    }

    // Handle handshake and get user
    let Ok(user) = handle_handshake(app.clone(), &mut ws_sink, &mut ws_stream).await else {
        ws_sink
            .reunite(ws_stream)
            .expect("WS sink and stream should be reuniteable")
            .close()
            .await
            .ok();
        return;
    };

    let conn_id = ConnectionId(user.id(), Uuid::new_v4());

    tracing::debug!(?user, "Connected: {} ({})", user.username(), conn_id);

    let (sender, receiver) = mpsc::unbounded_channel::<GatewayResponse>();
    let (broadcaster, _) = broadcast::channel::<GatewayMessage>(8);
    let broadcaster = Arc::new(broadcaster);

    let handle = SessionHandle::new(sender, broadcaster.clone());

    // Add user to peermap
    app.gateway().create_session(conn_id, handle);

    let user = user.include_presence(app.gateway()).await;
    let user_id = user.id();

    // We want to use the same sink in multiple tasks
    let ws_sink = Arc::new(Mutex::new(ws_sink));

    // Send READY and guild creates to user
    let send_onboarding = tokio::spawn(send_onboarding_payloads(app.clone(), user.clone(), ws_sink.clone()));

    // The tasks need to be dropped when their joinhandles are dropped by select!
    let send_events = tokio::spawn(send_events(
        user_id,
        UnboundedReceiverStream::new(receiver),
        ws_sink.clone(),
    ))
    .abort_on_drop();
    let receive_events = tokio::spawn(receive_events(conn_id, ws_stream, ws_sink, broadcaster.clone())).abort_on_drop();
    let handle_heartbeat = tokio::spawn(handle_heartbeating(
        broadcaster.clone(),
        app.clone(),
        conn_id,
        Duration::from_millis(HEARTBEAT_INTERVAL),
    ))
    .abort_on_drop();

    let is_server_shutting_down = tokio::select! {
        res = send_events => { matches!(res, Ok(Ok(GatewayCloseCode::GoingAway))) },
        _ = receive_events => { false },
        _ = handle_heartbeat => { false },
    };

    send_onboarding.abort();

    app.gateway().remove_session(conn_id);

    // If we're shutting down, don't spam out presence updates
    if is_server_shutting_down {
        return;
    }

    tracing::debug!(?user, "Disconnected: {} ({})", user.username(), conn_id);

    // Refetch presence in case it changed, to ensure we don't accidentally reveal the user's presence
    let presence = app.ops().fetch_presence(&user).await.expect("Failed to fetch presence");

    match presence {
        Presence::Offline => {}
        _ => {
            app.gateway().dispatch(
                GatewayEvent::PresenceUpdate {
                    user_id: user.id(),
                    presence: Presence::Offline,
                },
                SendMode::ToMutualGuilds(user.id()),
            );
        }
    }
}
