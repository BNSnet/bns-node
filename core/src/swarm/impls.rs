use async_trait::async_trait;
use rings_transport::core::transport::ConnectionInterface;

use crate::dht::Did;
use crate::error::Error;
use crate::error::Result;
use crate::measure::MeasureCounter;
use crate::message::ConnectNodeReport;
use crate::message::ConnectNodeSend;
use crate::message::Message;
use crate::message::MessagePayload;
use crate::message::PayloadSender;
use crate::swarm::Swarm;
use crate::types::Connection;

/// ConnectionHandshake defined how to connect two connections between two swarms.
#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
pub trait ConnectionHandshake {
    /// Create new connection and its offer.
    async fn prepare_connection_offer(&self, peer: Did) -> Result<(Connection, ConnectNodeSend)>;

    /// Answer the offer of remote connection.
    async fn answer_remote_connection(
        &self,
        peer: Did,
        offer_msg: &ConnectNodeSend,
    ) -> Result<(Connection, ConnectNodeReport)>;

    /// Accept the answer of remote connection.
    async fn accept_remote_connection(
        &self,
        peer: Did,
        answer_msg: &ConnectNodeReport,
    ) -> Result<Connection>;

    /// Creaet new connection and its answer. This function will wrap the offer inside a payload
    /// with verification.
    async fn create_offer(&self, peer: Did) -> Result<(Connection, MessagePayload<Message>)>;

    /// Answer the offer of remote connection. This function will verify the answer payload and
    /// will wrap the answer inside a payload with verification.
    async fn answer_offer(
        &self,
        offer_payload: MessagePayload<Message>,
    ) -> Result<(Connection, MessagePayload<Message>)>;

    /// Accept the answer of remote connection. This function will verify the answer payload and
    /// will return its did with the connection.
    async fn accept_answer(
        &self,
        answer_payload: MessagePayload<Message>,
    ) -> Result<(Did, Connection)>;
}

/// A trait for managing connections.
#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
pub trait ConnectionManager {
    /// Asynchronously disconnects the connection associated with the provided DID.
    async fn disconnect(&self, did: Did) -> Result<()>;

    /// Asynchronously establishes a new connection and returns the connection associated with the provided DID.
    async fn connect(&self, did: Did) -> Result<Connection>;

    /// Asynchronously establishes a new connection via a specified next hop DID and returns the connection associated with the provided DID.
    async fn connect_via(&self, did: Did, next_hop: Did) -> Result<Connection>;
}

/// A trait for judging whether a connection should be established with a given DID (Decentralized Identifier).
#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
pub trait Judegement {
    /// Asynchronously checks if a connection should be established with the provided DID.
    async fn should_connect(&self, did: Did) -> bool;

    /// Asynchronously records that a connection has been established with the provided DID.
    async fn record_connect(&self, did: Did);

    /// Asynchronously records that a connection has been disconnected with the provided DID.
    async fn record_disconnected(&self, did: Did);
}

/// A trait that combines the `Judegement` and `ConnectionManager` traits.
#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
pub trait JudgeConnection: Judegement + ConnectionManager {
    /// Asynchronously disconnects the connection associated with the provided DID after recording the disconnection.
    async fn disconnect(&self, did: Did) -> Result<()> {
        self.record_disconnected(did).await;
        tracing::debug!("[JudegeConnection] Disconnected {:?}", &did);
        ConnectionManager::disconnect(self, did).await
    }

    /// Asynchronously establishes a new connection and returns the connection associated with the provided DID if `should_connect` returns true; otherwise, returns an error.
    async fn connect(&self, did: Did) -> Result<Connection> {
        if !self.should_connect(did).await {
            return Err(Error::NodeBehaviourBad(did));
        }
        tracing::debug!("[JudgeConnection] Try Connect {:?}", &did);
        self.record_connect(did).await;
        ConnectionManager::connect(self, did).await
    }

    /// Asynchronously establishes a new connection via a specified next hop DID and returns the connection associated with the provided DID if `should_connect` returns true; otherwise, returns an error.
    async fn connect_via(&self, did: Did, next_hop: Did) -> Result<Connection> {
        if !self.should_connect(did).await {
            return Err(Error::NodeBehaviourBad(did));
        }
        tracing::debug!("[JudgeConnection] Try Connect {:?}", &did);
        self.record_connect(did).await;
        ConnectionManager::connect_via(self, did, next_hop).await
    }
}

impl Swarm {
    /// Record a succeeded message sent
    pub async fn record_sent(&self, did: Did) {
        if let Some(measure) = &self.measure {
            measure.incr(did, MeasureCounter::Sent).await;
        }
    }

    /// Record a failed message sent
    pub async fn record_sent_failed(&self, did: Did) {
        if let Some(measure) = &self.measure {
            measure.incr(did, MeasureCounter::FailedToSend).await;
        }
    }

    /// Check that a Did is behaviour good
    pub async fn behaviour_good(&self, did: Did) -> bool {
        if let Some(measure) = &self.measure {
            measure.good(did).await
        } else {
            true
        }
    }

    /// Create new connection that will be handled by swarm.
    pub async fn new_connection(&self, did: Did) -> Result<Connection> {
        self.backend
            .new_connection(did, self.transport_callback.clone())
            .await
    }
}

#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
impl ConnectionHandshake for Swarm {
    async fn prepare_connection_offer(&self, peer: Did) -> Result<(Connection, ConnectNodeSend)> {
        if self.backend.get_and_check_connection(peer).await.is_some() {
            return Err(Error::AlreadyConnected);
        };

        let conn = self.new_connection(peer).await?;

        let offer = conn.webrtc_create_offer().await.map_err(Error::Transport)?;
        let offer_str = serde_json::to_string(&offer).map_err(|_| Error::SerializeToString)?;
        let offer_msg = ConnectNodeSend { sdp: offer_str };

        Ok((conn, offer_msg))
    }

    async fn answer_remote_connection(
        &self,
        peer: Did,
        offer_msg: &ConnectNodeSend,
    ) -> Result<(Connection, ConnectNodeReport)> {
        if self.backend.get_and_check_connection(peer).await.is_some() {
            return Err(Error::AlreadyConnected);
        };

        let offer = serde_json::from_str(&offer_msg.sdp).map_err(Error::Deserialize)?;

        let conn = self.new_connection(peer).await?;
        let answer = conn
            .webrtc_answer_offer(offer)
            .await
            .map_err(Error::Transport)?;
        let answer_str = serde_json::to_string(&answer).map_err(|_| Error::SerializeToString)?;
        let answer_msg = ConnectNodeReport { sdp: answer_str };

        Ok((conn, answer_msg))
    }

    async fn accept_remote_connection(
        &self,
        peer: Did,
        answer_msg: &ConnectNodeReport,
    ) -> Result<Connection> {
        let answer = serde_json::from_str(&answer_msg.sdp).map_err(Error::Deserialize)?;

        let conn = self
            .backend
            .connection(peer)
            .ok_or(Error::ConnectionNotFound)?;
        conn.webrtc_accept_answer(answer)
            .await
            .map_err(Error::Transport)?;

        Ok(conn)
    }

    async fn create_offer(&self, peer: Did) -> Result<(Connection, MessagePayload<Message>)> {
        let (conn, offer_msg) = self.prepare_connection_offer(peer).await?;

        // This payload has fake next_hop.
        // The invoker should fix it before sending.
        let payload = MessagePayload::new_send(
            Message::ConnectNodeSend(offer_msg),
            self.session_sk(),
            self.did(),
            peer,
        )?;

        Ok((conn, payload))
    }

    async fn answer_offer(
        &self,
        offer_payload: MessagePayload<Message>,
    ) -> Result<(Connection, MessagePayload<Message>)> {
        if !offer_payload.verify() {
            return Err(Error::VerifySignatureFailed);
        }

        let Message::ConnectNodeSend(msg) = offer_payload.data else {
            return Err(Error::InvalidMessage(
                "Should be ConnectNodeSend".to_string(),
            ));
        };

        let peer = offer_payload.relay.origin_sender();
        let (conn, answer_msg) = self.answer_remote_connection(peer, &msg).await?;

        // This payload has fake next_hop.
        // The invoker should fix it before sending.
        let answer_payload = MessagePayload::new_send(
            Message::ConnectNodeReport(answer_msg),
            self.session_sk(),
            self.did(),
            self.did(),
        )?;

        Ok((conn, answer_payload))
    }

    async fn accept_answer(
        &self,
        answer_payload: MessagePayload<Message>,
    ) -> Result<(Did, Connection)> {
        tracing::debug!("accept_answer: {:?}", answer_payload);

        if !answer_payload.verify() {
            return Err(Error::VerifySignatureFailed);
        }

        let Message::ConnectNodeReport(ref msg) = answer_payload.data else {
            return Err(Error::InvalidMessage(
                "Should be ConnectNodeReport".to_string(),
            ));
        };

        let peer = answer_payload.relay.origin_sender();
        let conn = self.accept_remote_connection(peer, msg).await?;

        Ok((peer, conn))
    }
}

#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
impl ConnectionManager for Swarm {
    /// Disconnect a connection. There are three steps:
    /// 1) remove from DHT;
    /// 2) remove from Transport;
    /// 3) close the connection;
    async fn disconnect(&self, did: Did) -> Result<()> {
        self.backend.disconnect(did).await
    }

    /// Connect a given Did. If the did is already connected, return directly,
    /// else try prepare offer and establish connection by dht.
    /// This function may returns a pending connection or connected connection.
    async fn connect(&self, did: Did) -> Result<Connection> {
        tracing::info!("Try connect Did {:?}", &did);
        if let Some(t) = self.backend.get_and_check_connection(did).await {
            return Ok(t);
        }

        let conn = self.new_connection(did).await?;

        let offer = conn.webrtc_create_offer().await.map_err(Error::Transport)?;
        let offer_str = serde_json::to_string(&offer).map_err(|_| Error::SerializeToString)?;
        let offer_msg = ConnectNodeSend { sdp: offer_str };

        self.send_message(Message::ConnectNodeSend(offer_msg), did)
            .await?;

        Ok(conn)
    }

    /// Similar to connect, but this function will try connect a Did by given hop.
    async fn connect_via(&self, did: Did, next_hop: Did) -> Result<Connection> {
        if let Some(t) = self.backend.get_and_check_connection(did).await {
            return Ok(t);
        }

        tracing::info!("Try connect Did {:?}", &did);

        let conn = self.new_connection(did).await?;

        let offer = conn.webrtc_create_offer().await.map_err(Error::Transport)?;
        let offer_str = serde_json::to_string(&offer).map_err(|_| Error::SerializeToString)?;
        let offer_msg = ConnectNodeSend { sdp: offer_str };

        self.send_message_by_hop(Message::ConnectNodeSend(offer_msg), did, next_hop)
            .await?;

        Ok(conn)
    }
}

#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
impl Judegement for Swarm {
    /// Record a succeeded connected
    async fn record_connect(&self, did: Did) {
        if let Some(measure) = &self.measure {
            tracing::info!("[Judgement] Record connect");
            measure.incr(did, MeasureCounter::Connect).await;
        }
    }

    /// Record a disconnected
    async fn record_disconnected(&self, did: Did) {
        if let Some(measure) = &self.measure {
            tracing::info!("[Judgement] Record disconnected");
            measure.incr(did, MeasureCounter::Disconnected).await;
        }
    }

    /// Asynchronously checks if a connection should be established with the provided DID.
    async fn should_connect(&self, did: Did) -> bool {
        self.behaviour_good(did).await
    }
}
