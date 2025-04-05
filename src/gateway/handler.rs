use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::{self, Display, Formatter},
    sync::{Arc, Weak},
    time::Duration,
};

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
use http::StatusCode;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{
        Mutex,
        broadcast::{self, error::RecvError},
        mpsc::{self, error::SendError},
        oneshot,
    },
    time::timeout,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use crate::{
    app::{App, ApplicationState},
    models::{
        auth::Token,
        errors::GatewayError,
        gateway_event::{GatewayEvent, GatewayMessage, GuildCreatePayload},
        guild::Guild,
        snowflake::Snowflake,
        user::{Presence, User},
    },
    utils::join_handle::{AbortingJoinHandle, JoinHandleExt},
};

/// Default heartbeat interval in milliseconds
const HEARTBEAT_INTERVAL: u64 = 45000;

/// Possible responses issued by the server to a client
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum GatewayResponse {
    // If sent through a connection handle, the payload should be sent to the client
    Event(Arc<GatewayEvent>),
    // If sent through a connection handle, the connection should be closed
    Close(GatewayCloseCode, String),
}

/// Possible requests issued by the client to the server
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum GatewayRequest {
    // If sent through a connection handle, the payload should be broadcasted
    Message(GatewayMessage),
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
pub enum GatewayCloseCode {
    /// Successful operation / regular socket shutdown
    Normal = 1000,
    /// Client/Server is leaving (browser tab closing, server shutting down, etc.)
    GoingAway = 1001,
    /// Endpoint received a malformed frame
    ProtocolError = 1002,
    /// Endpoint received an unsupported frame (e.g. binary-only endpoint received text frame)
    Unsupported = 1003,
    /// Expected close status, received none
    NoStatus = 1005,
    /// No close code frame has been receieved
    Abnormal = 1006,
    /// Endpoint received inconsistent message (e.g. malformed UTF-8)
    InvalidPayload = 1007,
    ///Generic code used for situations other than 1003 and 1009
    PolicyViolation = 1008,
    /// Endpoint won't process large frame
    TooLarge = 1009,
    /// Client wanted an extension which server did not negotiate
    ExtensionRequired = 1010,
    /// Internal server error while operating
    ServerError = 1011,
    /// Server/service is restarting
    ServiceRestart = 1012,
    /// Temporary server condition forced blocking client's request
    TryAgainLater = 1013,
    /// Server acting as gateway received an invalid response
    BadGateway = 1014,
    /// Transport Layer Security handshake failure
    TLSHandshakeFail = 1015,
}

impl From<GatewayCloseCode> for u16 {
    fn from(value: GatewayCloseCode) -> Self {
        value as Self
    }
}

impl From<u16> for GatewayCloseCode {
    fn from(value: u16) -> Self {
        match value {
            1000 => Self::Normal,
            1001 => Self::GoingAway,
            1002 => Self::ProtocolError,
            1003 => Self::Unsupported,
            1005 => Self::NoStatus,
            1006 => Self::Abnormal,
            1007 => Self::InvalidPayload,
            1008 => Self::PolicyViolation,
            1009 => Self::TooLarge,
            1010 => Self::ExtensionRequired,
            1012 => Self::ServiceRestart,
            1013 => Self::TryAgainLater,
            1014 => Self::BadGateway,
            1015 => Self::TLSHandshakeFail,
            _ => Self::ServerError,
        }
    }
}

impl From<StatusCode> for GatewayCloseCode {
    fn from(value: StatusCode) -> Self {
        if !value.is_client_error() && !value.is_server_error() {
            return Self::Normal;
        }

        match value {
            StatusCode::PAYLOAD_TOO_LARGE | StatusCode::URI_TOO_LONG => Self::TooLarge,
            StatusCode::UNSUPPORTED_MEDIA_TYPE
            | StatusCode::RANGE_NOT_SATISFIABLE
            | StatusCode::EXPECTATION_FAILED
            | StatusCode::IM_A_TEAPOT
            | StatusCode::MISDIRECTED_REQUEST
            | StatusCode::HTTP_VERSION_NOT_SUPPORTED
            | StatusCode::VARIANT_ALSO_NEGOTIATES => Self::Unsupported,
            StatusCode::UNPROCESSABLE_ENTITY => Self::InvalidPayload,
            StatusCode::LOCKED
            | StatusCode::PRECONDITION_FAILED
            | StatusCode::LENGTH_REQUIRED
            | StatusCode::GONE
            | StatusCode::CONFLICT
            | StatusCode::REQUEST_TIMEOUT
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::NOT_ACCEPTABLE
            | StatusCode::PROXY_AUTHENTICATION_REQUIRED
            | StatusCode::NOT_FOUND
            | StatusCode::FORBIDDEN
            | StatusCode::UNAUTHORIZED
            | StatusCode::BAD_REQUEST
            | StatusCode::FAILED_DEPENDENCY
            | StatusCode::PRECONDITION_REQUIRED
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
            | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS => Self::PolicyViolation,
            StatusCode::BAD_GATEWAY => Self::BadGateway,
            StatusCode::SERVICE_UNAVAILABLE => Self::ServiceRestart,
            StatusCode::GATEWAY_TIMEOUT => Self::TryAgainLater,
            _ => Self::ServerError,
        }
    }
}

impl Serialize for GatewayCloseCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        (*self as u16).serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectionId(pub Snowflake<User>, pub Uuid);

impl Display for ConnectionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.0, self.1)
    }
}

/// A set of connection handles for a given user
///
/// ## Fields
///
/// * `user_id` - The ID of the user
/// * `guild_ids` - The guilds the user is a member of
/// * `handles` - The session handles for the user
/// * `broadcast` - The broadcast channel for incoming messages coming from sessions.
///    Session handles will forward messages to this channel.
#[derive(Debug)]
struct UserHandle {
    user_id: Snowflake<User>,
    guild_ids: HashSet<Snowflake<Guild>>,
    handles: HashMap<Uuid, SessionHandle>,
    broadcast: Arc<broadcast::Sender<(ConnectionId, GatewayMessage)>>,
}

impl UserHandle {
    pub fn new(user: impl Into<Snowflake<User>>, guild_ids: HashSet<Snowflake<Guild>>) -> Self {
        let (sender, _) = broadcast::channel(100);

        Self {
            user_id: user.into(),
            guild_ids,
            handles: HashMap::new(),
            broadcast: Arc::new(sender),
        }
    }

    /// Get the guilds the user is a member of
    pub const fn guild_ids(&self) -> &HashSet<Snowflake<Guild>> {
        &self.guild_ids
    }

    /// Get a mutable handle to the guilds the user is a member of
    pub fn guild_ids_mut(&mut self) -> &mut HashSet<Snowflake<Guild>> {
        &mut self.guild_ids
    }

    /// Subscribe to receive gateway messages from the user
    ///
    /// ## Returns
    ///
    /// A receiver for receiving messages from the user
    fn subscribe(&self) -> broadcast::Receiver<(ConnectionId, GatewayMessage)> {
        self.broadcast.subscribe()
    }

    /// Get a handle to the connection handle with the given ID
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection handle to get
    ///
    /// ## Returns
    ///
    /// A reference to the connection handle if it exists
    pub fn get_handle(&self, id: Uuid) -> Option<&SessionHandle> {
        self.handles.get(&id)
    }

    /// Iterate over all connection handles belonging to the user
    ///
    /// ## Returns
    ///
    /// An iterator over the connection handles
    pub fn iter_handles(&self) -> impl Iterator<Item = (&Uuid, &SessionHandle)> {
        self.handles.iter()
    }

    /// Get a mutable handle to the connection handle with the given ID
    /// If the handle does not exist, it is inserted and returned instead
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection handle to get
    /// * `handle` - The connection handle to insert if it does not exist
    ///
    /// ## Returns
    ///
    /// A mutable reference to the connection handle
    #[expect(dead_code)]
    pub fn get_or_add(&mut self, id: Uuid, mut default: SessionHandle) -> &mut SessionHandle {
        match self.handles.entry(id) {
            Entry::Vacant(v) => {
                default.bind_to(ConnectionId(self.user_id, id), &self.broadcast);
                v.insert(default)
            }
            Entry::Occupied(o) => o.into_mut(),
        }
    }

    /// Get a mutable handle to the connection handle with the given ID
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection handle to get
    ///
    /// ## Returns
    ///
    /// A mutable reference to the connection handle if it exists
    pub fn get_handle_mut(&mut self, id: Uuid) -> Option<&mut SessionHandle> {
        self.handles.get_mut(&id)
    }

    /// Insert a new connection handle with the given ID
    ///
    /// ## Arguments
    ///
    /// * `handle` - The connection handle to insert
    ///
    /// ## Returns
    ///
    /// The ID of the inserted connection handle
    pub fn add_session(&mut self, id: Uuid, mut handle: SessionHandle) -> Uuid {
        handle.bind_to(ConnectionId(self.user_id, id), &self.broadcast);
        self.handles.insert(id, handle);
        id
    }

    /// Close a connection handle with the given ID
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection handle to close
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    pub fn close_session(&mut self, id: Uuid, code: GatewayCloseCode, reason: String) {
        if let Some(handle) = self.handles.get_mut(&id) {
            if let Err(e) = handle.close(code, reason) {
                tracing::warn!(error = %e, "Failed to close connection handle");
            }
            self.handles.remove(&id);
        }
    }

    /// Drop a connection handle with the given ID
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection handle to drop
    pub fn drop_session(&mut self, id: Uuid) {
        self.handles.remove(&id);
    }

    /// Close all connection handles with the given code and reason
    ///
    /// ## Arguments
    ///
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    pub fn close_all(&mut self, code: GatewayCloseCode, reason: &str) {
        for handle in self.handles.values() {
            handle.close(code, reason.to_string()).ok();
        }
        self.handles.clear();
    }

    /// Check if the connection info is empty, meaning the user is not connected
    ///
    /// ## Returns
    ///
    /// `true` if the connection info is empty, `false` otherwise
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }
}

/// A struct representing a single session of a user
///
/// ## Fields
///
/// * `sender` - The sender for sending messages to the client
/// * `receiver` - The receiver for receiving messages from the client
/// * `guild_ids` - The guilds the user is a member of, this is used to filter events
#[derive(Debug)]
struct SessionHandle {
    /// Send messages to the client
    sender: mpsc::UnboundedSender<GatewayResponse>,
    /// Subscribe to this to receive events coming from the session
    receiver: Arc<broadcast::Sender<GatewayMessage>>,
    /// Use to forward events to the parent `ConnectionInfo`
    user_forwarder: Weak<broadcast::Sender<(ConnectionId, GatewayMessage)>>,
    /// The ID of this handle
    conn_id: Option<ConnectionId>,
    /// Handle to the forwarder task
    forwarder_task: Option<AbortingJoinHandle<()>>,
}

impl SessionHandle {
    /// Create a new connection handle with the given sender and guilds
    ///
    /// ## Arguments
    ///
    /// * `sender` - The sender for sending messages to the client
    /// * `receiver` - The receiver for receiving messages from the client
    pub const fn new(
        sender: mpsc::UnboundedSender<GatewayResponse>,
        receiver: Arc<broadcast::Sender<GatewayMessage>>,
    ) -> Self {
        Self {
            sender,
            receiver,
            conn_id: None,
            forwarder_task: None,
            user_forwarder: Weak::new(),
        }
    }

    /// Bind the connection handle to a `ConnectionInfo` and start forwarding messages to it.
    ///
    /// ## Arguments
    ///
    /// * `conn_id` - The ID this handle was assigned
    /// * `user_forwarder` - The forwarder to send messages to
    fn bind_to(
        &mut self,
        conn_id: ConnectionId,
        user_forwarder: &Arc<broadcast::Sender<(ConnectionId, GatewayMessage)>>,
    ) {
        self.user_forwarder = Arc::downgrade(user_forwarder);
        self.conn_id = Some(conn_id);
        self.start_forwarding();
    }

    fn start_forwarding(&mut self) {
        let mut receiver = self.receiver.subscribe();
        let Some(user_forwarder) = self.user_forwarder.upgrade() else {
            tracing::warn!("User forwarder is unavailable, requests from user will not be forwarded");
            return;
        };

        let Some(conn_id) = self.conn_id else {
            tracing::warn!("Connection ID is unavailable, requests from user will not be forwarded");
            return;
        };

        self.forwarder_task = Some(
            tokio::spawn(async move {
                loop {
                    match receiver.recv().await {
                        Ok(msg) => {
                            // Ignore potentially not having any receivers
                            user_forwarder.send((conn_id, msg)).ok();
                        }
                        Err(RecvError::Lagged(e)) => {
                            tracing::warn!(count = %e, "Forwarder is lagging, dropping messages");
                        }
                        Err(_) => break,
                    }
                }
            })
            .abort_on_drop(), // We don't want the task to leak if the handle is dropped */
        );
    }

    /// Send a message to the client
    ///
    /// ## Arguments
    ///
    /// * `message` - The message to send
    pub fn send(&self, message: Arc<GatewayEvent>) -> Result<(), SendError<GatewayResponse>> {
        let resp = GatewayResponse::Event(message);
        self.sender.send(resp)
    }

    /// Close the connection with the given code and reason
    /// This will also remove the handle from the gateway state
    ///
    /// ## Arguments
    ///
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    pub fn close(&self, code: GatewayCloseCode, reason: String) -> Result<(), SendError<GatewayResponse>> {
        let resp = GatewayResponse::Close(code, reason);
        self.sender.send(resp)
    }

    /// Subscribe to messages coming from the client
    ///
    /// ## Returns
    ///
    /// A receiver for receiving messages from the client
    pub fn subscribe(&self) -> broadcast::Receiver<GatewayMessage> {
        self.receiver.subscribe()
    }
}

/// Defines the possible modes for sending a message
#[derive(Debug, Clone, Copy)]
pub enum SendMode {
    /// Send the event to a specific user
    ToUser(Snowflake<User>),
    /// Send the event to all guilds the user is a member of
    ToMutualGuilds(Snowflake<User>),
    /// Send the event to all users in a guild
    ToGuild(Snowflake<Guild>),
}

enum Instruction {
    /// Dispatch a new event with the given send mode
    Dispatch(GatewayEvent, SendMode),
    /// Send an event to a specific user
    SendTo(Snowflake<User>, GatewayEvent),
    /// Send an event to a specific session of a user
    SendToSession(ConnectionId, GatewayEvent),
    /// Close a session with the given code and reason
    CloseSession(ConnectionId, GatewayCloseCode, String),
    /// Close all sessions from a user with the given code and reason
    CloseUser(Snowflake<User>, GatewayCloseCode, String),
    /// Close all sessions, shutting down the gateway
    CloseAll(oneshot::Sender<()>),
    /// Remove a session handle from the gateway state (This does not send a close frame)
    RemoveSession(ConnectionId),
    /// Add a new connection handle to the gateway state
    NewSession(ConnectionId, SessionHandle),
    /// Add a new guild member instance to an existing connection, if it exists
    AddMember(Snowflake<User>, Snowflake<Guild>),
    /// Remove a guild member instance from an existing connection, if it exists
    RemoveMember(Snowflake<User>, Snowflake<Guild>),
    /// Subscribe to receive messages from a specific user
    SubscribeToUser(
        Snowflake<User>,
        oneshot::Sender<Option<broadcast::Receiver<(ConnectionId, GatewayMessage)>>>,
    ),
    /// Subscribe to receive messages from a specific session
    SubscribeToSession(
        ConnectionId,
        oneshot::Sender<Option<broadcast::Receiver<GatewayMessage>>>,
    ),
    /// Query the connected status of a specific user
    QueryConnectedStatus(Snowflake<User>, oneshot::Sender<bool>),
    /// Query the connected status of multiple users
    /// The response will contain a set of users that are connected
    QueryMultiConnectedStatus(HashSet<Snowflake<User>>, oneshot::Sender<HashSet<Snowflake<User>>>),
}

#[derive(Debug)]
struct GatewayActor {
    receiver: mpsc::UnboundedReceiver<Instruction>,
    peermap: HashMap<Snowflake<User>, UserHandle>,
    app: Weak<ApplicationState>,
}

impl GatewayActor {
    fn new(app: Weak<ApplicationState>, receiver: mpsc::UnboundedReceiver<Instruction>) -> Self {
        Self {
            app,
            peermap: HashMap::new(),
            receiver,
        }
    }

    fn is_ready(&self) -> bool {
        self.app.upgrade().is_some()
    }

    fn app(&self) -> App {
        self.app
            .upgrade()
            .expect("GatewayActor is not bound to an ApplicationState")
    }

    pub async fn run(&mut self) {
        while let Some(instruction) = self.receiver.recv().await {
            if !self.is_ready() && !matches!(instruction, Instruction::CloseAll(_)) {
                tracing::warn!("App is not ready, ignoring instruction");
                continue;
            }

            match instruction {
                Instruction::NewSession(id, handle) => self.add_session(id, handle).await,
                Instruction::RemoveSession(id) => self.drop_session(id),
                Instruction::Dispatch(event, send_mode) => self.dispatch(event, send_mode),
                Instruction::SendTo(user, event) => self.send_to(user, event),
                Instruction::SendToSession(id, event) => self.send_to_session(id, event),
                Instruction::AddMember(user, guild) => self.add_member(user, guild),
                Instruction::RemoveMember(user, guild) => self.remove_member(user, guild),
                Instruction::CloseSession(conn, code, reason) => self.close_session(conn, code, reason),
                Instruction::CloseUser(user, code, reason) => self.close_user_sessions(user, code, &reason),
                Instruction::SubscribeToSession(id, tx) => {
                    let _ = tx.send(self.get_conn_recv(id));
                }
                Instruction::SubscribeToUser(id, tx) => {
                    let _ = tx.send(self.get_user_recv(id));
                }
                Instruction::QueryConnectedStatus(id, tx) => {
                    let _ = tx.send(self.is_connected(id));
                }
                Instruction::QueryMultiConnectedStatus(ids, tx) => {
                    let _ = tx.send(self.is_connected_multiple(ids));
                }
                Instruction::CloseAll(tx) => {
                    self.close();
                    let _ = tx.send(()); // Signal that the gateway has been closed
                    break;
                }
            }
        }
    }

    /// Add a new connection handle to the gateway state
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to add
    /// * `session` - The session handle to add
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    async fn add_session(&mut self, id: ConnectionId, session: SessionHandle) {
        if let Some(user_handle) = self.peermap.get_mut(&id.0) {
            user_handle.add_session(id.1, session);
        } else {
            let guild_ids = sqlx::query!(
                "SELECT guild_id FROM members WHERE user_id = $1",
                id.0 as Snowflake<User>
            )
            .fetch_all(self.app().db())
            .await
            .expect("Failed to fetch guilds during socket connection handling")
            .into_iter()
            .map(|row| row.guild_id.into())
            .collect::<HashSet<Snowflake<Guild>>>();

            let mut handle = UserHandle::new(id.0, guild_ids);
            handle.add_session(id.1, session);
            let mut receiver = handle.subscribe();
            let maybe_app = self.app.clone();

            // Call the default inbound handler
            tokio::spawn(async move {
                loop {
                    match receiver.recv().await {
                        Ok((id, msg)) => {
                            let Some(app) = maybe_app.upgrade() else { break };
                            tokio::spawn(async move {
                                app.ops().handle_inbound_gateway_message(id, msg).await;
                            });
                        }
                        Err(RecvError::Lagged(e)) => {
                            tracing::warn!(count = %e, "Global forwarder is lagging, dropping messages");
                        }
                        Err(_) => break,
                    }
                }
            });

            self.peermap.insert(id.0, handle);
        }
    }

    /// Remove a session handle from the gateway state
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to remove
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn drop_session(&mut self, id: ConnectionId) {
        if let Some(conn) = self.peermap.get_mut(&id.0) {
            conn.drop_session(id.1);

            if conn.is_empty() {
                self.peermap.remove(&id.0);
            }
        }
    }

    /// Get a receiver for receiving messages from a specific connection
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection to get the receiver for
    ///
    /// ## Returns
    ///
    /// A receiver for receiving messages from the connection, if it exists
    fn get_conn_recv(&self, id: ConnectionId) -> Option<broadcast::Receiver<GatewayMessage>> {
        Some(self.peermap.get(&id.0)?.get_handle(id.1)?.subscribe())
    }

    /// Get a receiver for receiving messages from a specific user
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to get the receiver for
    ///
    /// ## Returns
    ///
    /// A receiver for receiving messages from the user, if they are connected
    fn get_user_recv(&self, user_id: Snowflake<User>) -> Option<broadcast::Receiver<(ConnectionId, GatewayMessage)>> {
        Some(self.peermap.get(&user_id)?.subscribe())
    }

    /// Dispatch a new event originating from the given user to all other users
    ///
    /// ## Arguments
    ///
    /// * `payload` - The event payload
    fn dispatch(&mut self, event: GatewayEvent, send_mode: SendMode) {
        if let SendMode::ToUser(user_id) = send_mode {
            self.send_to(user_id, event);
            return;
        }

        tracing::debug!(?event, "Dispatching");

        let mut to_drop: Vec<ConnectionId> = Vec::new();

        // Avoid cloning the event for each user
        let event: Arc<GatewayEvent> = Arc::new(event);

        // Compute mutual guilds if the event is for mutual guilds
        let event_user_id = if let SendMode::ToMutualGuilds(user_id) = send_mode {
            Some(user_id)
        } else {
            None
        };

        let event_user_guilds = event_user_id.and_then(|uid| self.peermap.get(&uid).map(|a| a.guild_ids().clone()));

        // TODO: if event is a GUILD_REMOVE, remove the guild from guild sets

        for (uid, conninfo) in &mut self.peermap {
            for (handle_id, handle) in conninfo.iter_handles() {
                // If the event is guild-specific, only send it to users that are members of that guild
                if let SendMode::ToGuild(event_guild) = send_mode {
                    if !conninfo.guild_ids().contains(&event_guild) {
                        continue;
                    }
                }
                // Avoid sending events to users that don't share any guilds with the event originator
                else if let Some(ref guild_ids) = event_user_guilds {
                    if guild_ids.intersection(conninfo.guild_ids()).next().is_none() {
                        continue;
                    }
                }

                if let Err(err) = handle.send(event.clone()) {
                    tracing::warn!(error = %err, "Error dispatching event to user: {uid}");
                    to_drop.push(ConnectionId(*uid, *handle_id));
                }
            }
        }

        for conn in to_drop {
            self.drop_session(conn);
        }
    }

    /// Send an event to a specific user. If they are not connected, the event is dropped.
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to send the event to
    /// * `event` - The event to send
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn send_to(&mut self, user: impl Into<Snowflake<User>>, event: GatewayEvent) {
        let user_id = user.into();
        let event = Arc::new(event);
        let mut to_drop: Vec<ConnectionId> = Vec::new();
        let Some(conn) = self.peermap.get(&user_id) else { return };

        for (handle_id, handle) in conn.iter_handles() {
            if let Err(err) = handle.send(event.clone()) {
                tracing::warn!(error = %err, "Error sending event to session: {}-{}", &user_id, handle_id);
                to_drop.push(ConnectionId(user_id, *handle_id));
            }
        }

        for conn in to_drop {
            self.drop_session(conn);
        }
    }

    /// Send an event to a specific user session. If the session is not connected, the event is dropped.
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the session to send the event to
    /// * `event` - The event to send
    fn send_to_session(&mut self, id: ConnectionId, event: GatewayEvent) {
        let event = Arc::new(event);
        let Some(conn) = self.peermap.get_mut(&id.0) else {
            return;
        };
        let Some(handle) = conn.get_handle_mut(id.1) else {
            return;
        };

        if let Err(err) = handle.send(event) {
            tracing::warn!(error = %err, "Error sending event to session: {id}");
            self.drop_session(id);
        }
    }

    /// Close a session with the given code and reason
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to drop
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    fn close_session(&mut self, conn_id: ConnectionId, code: GatewayCloseCode, reason: String) {
        if let Some(user_handle) = self.peermap.get_mut(&conn_id.0) {
            user_handle.close_session(conn_id.1, code, reason);

            if user_handle.is_empty() {
                self.peermap.remove(&conn_id.0);
            }
        }
    }

    /// Close all sessions from a user with the given code and reason
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to drop
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    fn close_user_sessions(&mut self, user: impl Into<Snowflake<User>>, code: GatewayCloseCode, reason: &str) {
        let user_id = user.into();
        if let Some(conn) = self.peermap.get_mut(&user_id) {
            conn.close_all(code, reason);
            self.peermap.remove(&user_id);
        }
    }

    fn close(&mut self) {
        for conn in self.peermap.values_mut() {
            conn.close_all(GatewayCloseCode::GoingAway, "Server shutting down");
        }
        self.peermap.clear();
    }

    /// Determines if the given user is connected
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to check for
    ///
    /// ## Returns
    ///
    /// `true` if the user is connected, `false` otherwise
    fn is_connected(&self, user: impl Into<Snowflake<User>>) -> bool {
        self.peermap.get(&user.into()).is_some_and(|conn| !conn.is_empty())
    }

    /// Filter out users that are not connected
    ///
    /// ## Arguments
    ///
    /// * `users` - The users to filter
    ///
    /// ## Returns
    ///
    /// A set of users that are connected
    fn is_connected_multiple(&self, users: HashSet<Snowflake<User>>) -> HashSet<Snowflake<User>> {
        users
            .into_iter()
            .filter(|u| self.peermap.get(u).is_some_and(|conn| !conn.is_empty()))
            .collect()
    }

    /// Registers a new guild member instance to an existing connection
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to add to the connection
    /// * `guild` - The guild to add the user to
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn add_member(&mut self, user: impl Into<Snowflake<User>>, guild: impl Into<Snowflake<Guild>>) {
        if let Some(handle) = self.peermap.get_mut(&user.into()) {
            handle.guild_ids_mut().insert(guild.into());
        }
    }

    /// Removes a guild member instance from an existing connection
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to remove from the connection
    /// * `guild` - The guild to remove the user from
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn remove_member(&mut self, user: impl Into<Snowflake<User>>, guild: impl Into<Snowflake<Guild>>) {
        if let Some(handle) = self.peermap.get_mut(&user.into()) {
            handle.guild_ids_mut().remove(&guild.into());
        }
    }
}

/// The gateway actor handle that can be used to manage gateway state.
///
/// All operations are queued to be executed on the internal gateway actor,
/// and thus are not async, unless they require a response.
#[derive(Debug)]
pub struct Gateway {
    sender: Option<mpsc::UnboundedSender<Instruction>>,
    task: Option<tokio::task::JoinHandle<()>>,
    app: Weak<ApplicationState>,
    is_bound: bool,
}

impl Gateway {
    pub const fn new() -> Self {
        Self {
            sender: None,
            task: None,
            app: Weak::new(),
            is_bound: false,
        }
    }

    pub fn bind_to(&mut self, app: Weak<ApplicationState>) {
        self.app = app;
        self.is_bound = true;
    }

    /// Start the gateway
    ///
    /// Starts the internal gateway actor and begins processing messages
    pub fn start(&mut self) {
        assert!(self.is_bound, "Gateway was not bound to an application state");

        if let Some(a) = self.task.as_ref() {
            a.abort();
        }

        let (sender, receiver) = mpsc::unbounded_channel();

        let mut inner = GatewayActor::new(self.app.clone(), receiver);

        self.task = Some(tokio::spawn(async move {
            inner.run().await;
        }));
        self.sender = Some(sender);
    }

    /// Gracefully stop the gateway.
    ///
    /// This sends a close request to the gateway actor and waits for it to close.
    pub async fn stop(&self) {
        let Some(sender) = &self.sender else { return };

        let (tx, rx) = oneshot::channel();

        if let Err(e) = sender.send(Instruction::CloseAll(tx)) {
            tracing::error!(error = %e, "Failed to send close instruction to gateway");
            return;
        }
        if let Err(e) = rx.await {
            tracing::error!(error = %e, "Failed to close gateway");
        }
    }

    /// Abort the gateway.
    ///
    /// This will immediately stop the gateway without waiting for it to close.
    pub fn abort(&mut self) {
        if let Some(a) = self.task.take() {
            a.abort();
        }
    }

    pub const fn is_started(&self) -> bool {
        self.sender.is_some() && self.task.is_some()
    }

    /// Send an instruction to the inner actor
    ///
    /// ## Arguments
    ///
    /// * `instruction` - The instruction to send
    fn send_instruction(&self, instruction: Instruction) {
        if let Some(sender) = &self.sender {
            sender.send(instruction).expect("Failed to send instruction to gateway");
        } else {
            panic!("Gateway is not running");
        }
    }

    /// Get the connection receiver for the given connection ID
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection to get the receiver for
    ///
    /// ## Returns
    ///
    /// A receiver for receiving gateway messages from the connection,
    /// or `None` if the connection does not exist
    pub async fn get_conn_recv(&self, id: ConnectionId) -> Option<broadcast::Receiver<GatewayMessage>> {
        let (tx, rx) = oneshot::channel();
        self.send_instruction(Instruction::SubscribeToSession(id, tx));

        match rx.await {
            Ok(Some(receiver)) => Some(receiver),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(error = %e, "Failed to subscribe to connection");
                None
            }
        }
    }

    /// Get the user receiver for the given user ID
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to get the receiver for
    ///
    /// ## Returns
    ///
    /// A receiver for receiving gateway messages from the user,
    /// or `None` if the user has no active connections
    pub async fn get_user_recv(
        &self,
        user_id: Snowflake<User>,
    ) -> Option<broadcast::Receiver<(ConnectionId, GatewayMessage)>> {
        let (tx, rx) = oneshot::channel();
        self.send_instruction(Instruction::SubscribeToUser(user_id, tx));

        match rx.await {
            Ok(Some(receiver)) => Some(receiver),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(error = %e, "Failed to subscribe to user");
                None
            }
        }
    }

    /// Add a new connection handle to the gateway state
    ///
    /// ## Arguments
    ///
    /// * `id` - The composite session ID to bind this session to.
    ///   The first part of the ID should be the user it belongs to,
    ///   the second part should be a unique identifier of this session.
    /// * `handle` - The connection handle to add
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn create_session(&self, id: ConnectionId, handle: SessionHandle) {
        self.send_instruction(Instruction::NewSession(id, handle));
    }

    /// Removes a session with the given ID
    ///
    /// Note that this does not close the connection, it only removes the session from the gateway state.
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the session to remove
    fn remove_session(&self, id: ConnectionId) {
        self.send_instruction(Instruction::RemoveSession(id));
    }

    /// Dispatch a new event originating from the given user to all other users
    ///
    /// ## Arguments
    ///
    /// * `payload` - The event payload
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    pub fn dispatch(&self, event: GatewayEvent, send_mode: SendMode) {
        self.send_instruction(Instruction::Dispatch(event, send_mode));
    }

    /// Close a user session with the given code and reason
    /// This will also remove the handle from the gateway state
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to drop
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    pub fn close_session(&self, conn: ConnectionId, code: GatewayCloseCode, reason: String) {
        self.send_instruction(Instruction::CloseSession(conn, code, reason));
    }

    /// Close all user sessions with the given code and reason
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to drop
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    pub fn close_all_user_sessions(&self, user: impl Into<Snowflake<User>>, code: GatewayCloseCode, reason: String) {
        self.send_instruction(Instruction::CloseUser(user.into(), code, reason));
    }

    /// Registers a new guild member instance to an existing connection
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to add to the connection
    /// * `guild` - The guild to add the user to
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    pub fn add_member(&self, user: impl Into<Snowflake<User>>, guild: impl Into<Snowflake<Guild>>) {
        self.send_instruction(Instruction::AddMember(user.into(), guild.into()));
    }

    /// Removes a guild member instance from an existing connection
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to remove from the connection
    /// * `guild` - The guild to remove the user from
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    pub fn remove_member(&self, user: impl Into<Snowflake<User>>, guild: impl Into<Snowflake<Guild>>) {
        self.send_instruction(Instruction::RemoveMember(user.into(), guild.into()));
    }

    /// Send an event to a specific user. If they are not connected, the event is dropped.
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to send the event to
    /// * `event` - The event to send
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    pub fn send_to(&self, user: impl Into<Snowflake<User>>, event: GatewayEvent) {
        self.send_instruction(Instruction::SendTo(user.into(), event));
    }

    /// Send an event to a specific session. If the session is not connected, the event is dropped.
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the session to send the event to
    /// * `event` - The event to send
    pub fn send_to_session(&self, id: ConnectionId, event: GatewayEvent) {
        self.send_instruction(Instruction::SendToSession(id, event));
    }

    /// Returns whether the given user is connected
    ///
    /// ## Arguments
    ///
    /// * `user` - The user to check for
    ///
    /// ## Returns
    ///
    /// `true` if the user is connected, `false` otherwise
    pub async fn is_connected(&self, user: impl Into<Snowflake<User>>) -> bool {
        let (tx, rx) = oneshot::channel();
        self.send_instruction(Instruction::QueryConnectedStatus(user.into(), tx));
        rx.await.expect("Failed to query connection status")
    }

    /// Returns a set of users that are connected
    /// from the given set of users
    ///
    /// ## Arguments
    /// * `users` - The users to check for
    ///
    /// ## Returns
    ///
    /// A set of users that are connected
    pub async fn is_connected_multiple(&self, users: HashSet<Snowflake<User>>) -> HashSet<Snowflake<User>> {
        if users.is_empty() {
            return users;
        }

        let (tx, rx) = oneshot::channel();
        self.send_instruction(Instruction::QueryMultiConnectedStatus(users, tx));
        rx.await.expect("Failed to query connection status")
    }
}

impl Default for Gateway {
    fn default() -> Self {
        Self::new()
    }
}

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
