use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
    time::Duration,
};

use axum::{
    extract::{
        ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{
        broadcast,
        mpsc::{self, error::SendError},
        oneshot, Mutex,
    },
    time::timeout,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use crate::{
    models::{
        auth::Token,
        errors::GatewayError,
        gateway_event::{
            EventLike, GatewayEvent, GatewayMessage, GuildCreatePayload, HelloPayload, PresenceUpdatePayload,
            ReadyPayload,
        },
        guild::Guild,
        snowflake::Snowflake,
        state::{App, ApplicationState},
        user::{Presence, User},
    },
    utils::join_handle::JoinHandleExt,
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

#[derive(Debug, Clone)]
struct ConnectionInfo {
    guild_ids: HashSet<Snowflake<Guild>>,
    handles: HashMap<Uuid, ConnectionHandle>,
}

impl ConnectionInfo {
    pub const fn new(guild_ids: HashSet<Snowflake<Guild>>, handles: HashMap<Uuid, ConnectionHandle>) -> Self {
        Self { guild_ids, handles }
    }

    /// Get the guilds the user is a member of
    pub const fn guild_ids(&self) -> &HashSet<Snowflake<Guild>> {
        &self.guild_ids
    }

    /// Get a mutable handle to the guilds the user is a member of
    pub fn guild_ids_mut(&mut self) -> &mut HashSet<Snowflake<Guild>> {
        &mut self.guild_ids
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
    #[expect(dead_code)]
    pub fn get_handle(&self, id: Uuid) -> Option<&ConnectionHandle> {
        self.handles.get(&id)
    }

    /// Iterate over all connection handles belonging to the user
    ///
    /// ## Returns
    ///
    /// An iterator over the connection handles
    pub fn iter_handles(&self) -> impl Iterator<Item = (&Uuid, &ConnectionHandle)> {
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
    pub fn get_or_insert(&mut self, id: Uuid, default: ConnectionHandle) -> &mut ConnectionHandle {
        self.handles.entry(id).or_insert(default)
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
    pub fn get_handle_mut(&mut self, id: Uuid) -> Option<&mut ConnectionHandle> {
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
    pub fn insert_handle(&mut self, id: Uuid, handle: ConnectionHandle) -> Uuid {
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
    pub fn close_handle(&mut self, id: Uuid, code: GatewayCloseCode, reason: String) {
        if let Some(handle) = self.handles.get_mut(&id) {
            handle.close(code, reason).ok();
            self.handles.remove(&id);
        }
    }

    /// Drop a connection handle with the given ID
    ///
    /// ## Arguments
    ///
    /// * `id` - The ID of the connection handle to drop
    pub fn drop_handle(&mut self, id: Uuid) {
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

/// A struct containing connection details for a user
///
/// ## Fields
///
/// * `sender` - The sender for sending messages to the client
/// * `receiver` - The receiver for receiving messages from the client
/// * `guild_ids` - The guilds the user is a member of, this is used to filter events
#[derive(Debug, Clone)]
struct ConnectionHandle {
    sender: mpsc::UnboundedSender<GatewayResponse>,
}

impl ConnectionHandle {
    /// Create a new connection handle with the given sender and guilds
    ///
    /// ## Arguments
    ///
    /// * `sender` - The sender for sending messages to the client
    /// * `receiver` - The receiver for receiving messages from the client
    pub const fn new(sender: mpsc::UnboundedSender<GatewayResponse>) -> Self {
        Self { sender }
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
}

#[derive(Debug)]
pub enum SendMode {
    ToUser(Snowflake<User>),
    ToUserGuilds(Snowflake<User>, HashSet<Snowflake<Guild>>),
    ToGuild(Snowflake<Guild>),
}

enum Instruction {
    Dispatch(GatewayEvent),
    SendTo(Snowflake<User>, GatewayEvent),
    SendToSession(ConnectionId, GatewayEvent),
    CloseSession(ConnectionId, GatewayCloseCode, String),
    CloseUser(Snowflake<User>, GatewayCloseCode, String),
    CloseAll(oneshot::Sender<()>),
    RemoveSession(ConnectionId),
    NewSession(ConnectionId, ConnectionHandle),
    AddMember(Snowflake<User>, Snowflake<Guild>),
    RemoveMember(Snowflake<User>, Snowflake<Guild>),
    QueryConnectedStatus(Snowflake<User>, oneshot::Sender<bool>),
}

#[derive(Debug)]
struct GatewayInner {
    receiver: mpsc::UnboundedReceiver<Instruction>,
    peers: HashMap<Snowflake<User>, ConnectionInfo>,
    app: Weak<ApplicationState>,
}

impl GatewayInner {
    fn new(app: Weak<ApplicationState>, receiver: mpsc::UnboundedReceiver<Instruction>) -> Self {
        Self {
            app,
            peers: HashMap::new(),
            receiver,
        }
    }

    fn app(&self) -> App {
        self.app
            .upgrade()
            .expect("GatewayInner is not bound to an ApplicationState")
    }

    pub async fn run(&mut self) {
        while let Some(instruction) = self.receiver.recv().await {
            match instruction {
                Instruction::NewSession(id, handle) => self.add_handle(id, handle).await,
                Instruction::RemoveSession(id) => self.remove_handle(id),
                Instruction::Dispatch(event) => self.dispatch(event),
                Instruction::SendTo(user, event) => self.send_to(user, event),
                Instruction::SendToSession(id, event) => self.send_to_session(id, event),
                Instruction::AddMember(user, guild) => self.add_member(user, guild),
                Instruction::RemoveMember(user, guild) => self.remove_member(user, guild),
                Instruction::CloseSession(conn, code, reason) => self.close_session(conn, code, reason),
                Instruction::CloseUser(user, code, reason) => self.close_user_sessions(user, code, &reason),
                Instruction::QueryConnectedStatus(id, tx) => {
                    let _ = tx.send(self.is_connected(id));
                }
                Instruction::CloseAll(tx) => {
                    self.close();
                    let _ = tx.send(()); // Notify the caller that the gateway has been closed
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
    /// * `handle` - The connection handle to add
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    async fn add_handle(&mut self, id: ConnectionId, handle: ConnectionHandle) {
        if let Some(conn) = self.peers.get_mut(&id.0) {
            conn.insert_handle(id.1, handle);
        } else {
            let mut handles = HashMap::new();

            let guild_ids = sqlx::query!(
                "SELECT guild_id FROM members WHERE user_id = $1",
                id.0 as Snowflake<User>
            )
            .fetch_all(self.app().db.pool())
            .await
            .expect("Failed to fetch guilds during socket connection handling")
            .into_iter()
            .map(|row| row.guild_id.into())
            .collect::<HashSet<Snowflake<Guild>>>();

            handles.insert(id.1, handle);
            self.peers.insert(id.0, ConnectionInfo::new(guild_ids, handles));
        }
    }

    /// Remove a connection handle from the gateway state
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to remove
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn remove_handle(&mut self, id: ConnectionId) {
        if let Some(conn) = self.peers.get_mut(&id.0) {
            conn.drop_handle(id.1);

            if conn.is_empty() {
                self.peers.remove(&id.0);
            }
        }
    }

    /// Dispatch a new event originating from the given user to all other users
    ///
    /// ## Arguments
    ///
    /// * `payload` - The event payload
    fn dispatch(&mut self, event: GatewayEvent) {
        tracing::debug!(?event, "Dispatching event");

        let mut to_drop: Vec<ConnectionId> = Vec::new();

        // Avoid cloning the event for each user
        let event: Arc<GatewayEvent> = Arc::new(event);
        let event_guild_id = event.extract_guild_id();
        let event_user_id = event.extract_user_id();
        let event_user_guilds = event_user_id.and_then(|uid| self.peers.get(&uid).map(|a| a.guild_ids().clone()));

        for (uid, conninfo) in &mut self.peers {
            for (handle_id, handle) in conninfo.iter_handles() {
                // If the event is guild-specific, only send it to users that are members of that guild
                if let Some(event_guild) = event_guild_id {
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
            self.remove_handle(conn);
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
        let Some(conn) = self.peers.get(&user_id) else { return };

        for (handle_id, handle) in conn.iter_handles() {
            if let Err(err) = handle.send(event.clone()) {
                tracing::warn!(error = %err, "Error sending event to session: {}-{}", &user_id, handle_id);
                to_drop.push(ConnectionId(user_id, *handle_id));
            }
        }

        for conn in to_drop {
            self.remove_handle(conn);
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
        let Some(conn) = self.peers.get_mut(&id.0) else { return };
        let Some(handle) = conn.get_handle_mut(id.1) else {
            return;
        };

        if let Err(err) = handle.send(event) {
            tracing::warn!(error = %err, "Error sending event to session: {}-{}", &id.0, id.1);
            self.remove_handle(id);
        }
    }

    /// Close a session with the given code and reason
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to drop
    /// * `code` - The close code to send
    /// * `reason` - The reason for closing the connection
    fn close_session(&mut self, conn: ConnectionId, code: GatewayCloseCode, reason: String) {
        self.peers.entry(conn.0).and_modify(|a| {
            a.close_handle(conn.1, code, reason);
        });
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
        if let Some(conn) = self.peers.get_mut(&user_id) {
            conn.close_all(code, reason);
            self.peers.remove(&user_id);
        }
    }

    fn close(&mut self) {
        for conn in self.peers.values_mut() {
            conn.close_all(GatewayCloseCode::GoingAway, "Server shutting down");
        }
        self.peers.clear();
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
        self.peers.get(&user.into()).is_some_and(|conn| !conn.is_empty())
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
        if let Some(handle) = self.peers.get_mut(&user.into()) {
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
        if let Some(handle) = self.peers.get_mut(&user.into()) {
            handle.guild_ids_mut().remove(&guild.into());
        }
    }
}

/// A singleton representing the gateway state
#[derive(Debug)]
pub struct Gateway {
    sender: Option<mpsc::UnboundedSender<Instruction>>,
    task: Option<tokio::task::JoinHandle<()>>,
    app: Weak<ApplicationState>,
}

impl Gateway {
    pub const fn new() -> Self {
        Self {
            sender: None,
            task: None,
            app: Weak::new(),
        }
    }

    pub fn bind_to(&mut self, app: Weak<ApplicationState>) {
        self.app = app;
    }

    pub fn start(&mut self) {
        let (sender, receiver) = mpsc::unbounded_channel();

        let mut inner = GatewayInner::new(self.app.clone(), receiver);

        self.task = Some(tokio::spawn(async move {
            inner.run().await;
        }));
        self.sender = Some(sender);
    }

    pub async fn stop(&self) {
        let Some(sender) = &self.sender else { return };

        let (tx, rx) = oneshot::channel();

        if let Err(e) = sender.send(Instruction::CloseAll(tx)) {
            tracing::error!(error = %e, "Failed to send close instruction to gateway");
            return;
        }
        rx.await.expect("Failed to close gateway");
    }

    pub fn abort(&mut self) {
        if let Some(a) = self.task.take() {
            a.abort();
        }
    }

    fn send_instruction(&self, instruction: Instruction) {
        self.sender.as_ref().map(|a| a.send(instruction));
    }

    /// Add a new connection handle to the gateway state
    ///
    /// ## Arguments
    ///
    /// * `user_id` - The ID of the user to add
    /// * `handle` - The connection handle to add
    ///
    /// ## Locks
    ///
    /// * `peers` (write)
    fn create_session(&self, id: ConnectionId, handle: ConnectionHandle) {
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
    pub fn dispatch(&self, event: GatewayEvent) {
        self.send_instruction(Instruction::Dispatch(event));
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
    Router::new().route("/", get(websocket_handler))
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
/// ## Returns
///
/// `Ok(())` if the message was sent successfully, an error otherwise
async fn send_close_frame(
    ws_sink: &mut SplitSink<WebSocket, Message>,
    code: GatewayCloseCode,
    reason: impl Into<Utf8Bytes>,
) -> Result<(), axum::Error> {
    ws_sink
        .send(Message::Close(Some(CloseFrame {
            code: code.into(),
            reason: reason.into(),
        })))
        .await
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
            serde_json::to_string(&GatewayEvent::Hello(HelloPayload::new(HEARTBEAT_INTERVAL)))
                .expect("Failed to serialize HELLO payload")
                .into(),
        ))
        .await
        .ok();

    // IDENTIFY should be the first message sent
    let Ok(Some(Ok(ident))) = timeout(Duration::from_secs(5), ws_stream.next()).await else {
        send_close_frame(ws_sink, GatewayCloseCode::PolicyViolation, "IDENTIFY expected").await?;
        return Err(GatewayError::HandshakeFailure("IDENTIFY expected".into()));
    };

    let Message::Text(text) = ident else {
        send_close_frame(ws_sink, GatewayCloseCode::InvalidPayload, "Invalid IDENTIFY payload").await?;
        return Err(GatewayError::MalformedFrame("Invalid IDENTIFY payload".into()));
    };

    let Ok(GatewayMessage::Identify(payload)) = serde_json::from_str(&text) else {
        send_close_frame(ws_sink, GatewayCloseCode::InvalidPayload, "Invalid IDENTIFY payload").await?;
        return Err(GatewayError::MalformedFrame("Invalid IDENTIFY payload".into()));
    };

    let Ok(token) = Token::validate(app.clone(), payload.token.expose_secret()).await else {
        send_close_frame(ws_sink, GatewayCloseCode::PolicyViolation, "Invalid token").await?;
        return Err(GatewayError::AuthError("Invalid token".into()));
    };

    let user_id = token.data().user_id();
    let Some(user) = app.ops().fetch_user(user_id).await else {
        send_close_frame(ws_sink, GatewayCloseCode::ServerError, "No user belongs to token").await?;
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

            app.gateway.close_session(id, close_code, reason);
            break;
        }

        app.gateway.send_to_session(id, GatewayEvent::HeartbeatAck);
    }
}

/// Send the `READY` event, all `GUILD_CREATE` events, and dispatch a `PRESENCE_UPDATE` event for this user
///
/// ## Arguments
///
/// * `app` - The shared application state
/// * `user` - The user to send the `READY` event to
/// * `ws_sink` - The sink for sending messages to the user
async fn send_ready(
    app: App,
    user: User,
    ws_sink: Arc<Mutex<SplitSink<WebSocket, Message>>>,
) -> Result<(), axum::Error> {
    let guilds = app
        .ops()
        .fetch_guilds_for(&user)
        .await
        .expect("Failed to fetch guilds during socket connection handling");

    // Send READY
    send_serializable(
        &mut *ws_sink.lock().await,
        GatewayEvent::Ready(ReadyPayload::new(user.clone(), guilds.clone())),
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
            app.gateway
                .dispatch(GatewayEvent::PresenceUpdate(PresenceUpdatePayload {
                    user_id: user.id(),
                    presence: *user.last_presence(),
                }));
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
                send_close_frame(&mut *ws_sink.lock().await, code, reason).await.ok();
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
    user_id: Snowflake<User>,
    mut ws_stream: SplitStream<WebSocket>,
    ws_sink: Arc<Mutex<SplitSink<WebSocket, Message>>>,
    broadcaster: Arc<broadcast::Sender<GatewayMessage>>,
) {
    while let Some(msg) = ws_stream.next().await {
        // Close if the user sends a close frame
        if let Ok(Message::Close(f)) = msg {
            tracing::debug!(close_frame = ?f, "Gateway stream closed by {user_id}: {f:?}");
            break;
        }
        // Otherwise attempt to parse the message and send it
        let Ok(Message::Text(text)) = msg else {
            send_close_frame(
                &mut *ws_sink.lock().await,
                GatewayCloseCode::Unsupported,
                "Unsupported message encoding",
            )
            .await
            .ok();
            break;
        };

        match serde_json::from_str::<GatewayRequest>(&text) {
            Ok(req) => {
                let GatewayRequest::Message(msg) = req;
                broadcaster.send(msg).ok();
            }
            Err(e) => {
                send_close_frame(
                    &mut *ws_sink.lock().await,
                    GatewayCloseCode::InvalidPayload,
                    format!("Invalid request payload: {e}"),
                )
                .await
                .ok();
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

    tracing::debug!(?user, "Connected: {} ({})", user.username(), user.id());

    let (sender, receiver) = mpsc::unbounded_channel::<GatewayResponse>();
    let (broadcaster, _) = broadcast::channel::<GatewayMessage>(100);
    let broadcaster = Arc::new(broadcaster);

    let handle = ConnectionHandle::new(sender);
    let conn_id = ConnectionId(user.id(), Uuid::new_v4());

    // Add user to peermap
    app.gateway.create_session(conn_id, handle);

    let user = user.include_presence(&app.gateway).await;
    let user_id = user.id();

    // We want to use the same sink in multiple tasks, so we wrap it in an Arc<Mutex>
    let ws_sink = Arc::new(Mutex::new(ws_sink));

    // Send READY and guild creates to user
    let send_ready = tokio::spawn(send_ready(app.clone(), user.clone(), ws_sink.clone()));

    // The tasks need to be dropped when their joinhandles are dropped by select!
    let send_events = tokio::spawn(send_events(
        user_id,
        UnboundedReceiverStream::new(receiver),
        ws_sink.clone(),
    ))
    .abort_on_drop();
    let receive_events = tokio::spawn(receive_events(user_id, ws_stream, ws_sink, broadcaster.clone())).abort_on_drop();
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

    send_ready.abort();

    // If we're shutting down, don't spam out presence updates
    if is_server_shutting_down {
        return;
    }

    // Disconnection logic
    app.gateway.remove_session(conn_id);
    tracing::debug!(?user, "Disconnected: {} ({})", user.username(), user.id());

    // Refetch presence in case it changed
    let presence = app.ops().fetch_presence(&user).await.expect("Failed to fetch presence");

    // Send presence update to OFFLINE
    match presence {
        Presence::Offline => {}
        _ => {
            app.gateway
                .dispatch(GatewayEvent::PresenceUpdate(PresenceUpdatePayload {
                    user_id: user.id(),
                    presence: Presence::Offline,
                }));
        }
    }
}
