//! Tranposrt managerment
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;

use async_stream::stream;
use async_trait::async_trait;
use futures::Stream;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::channels::Channel;
use crate::dht::Did;
use crate::dht::PeerRing;
use crate::ecc::SecretKey;
use crate::err::Error;
use crate::err::Result;
use crate::message;
use crate::message::handlers::CallbackFn;
use crate::message::Decoder;
use crate::message::Encoder;
use crate::message::Message;
use crate::message::MessagePayload;
use crate::message::PayloadSender;
use crate::session::SessionManager;
use crate::session::Ttl;
use crate::storage::MemStorage;
use crate::storage::PersistenceStorage;
use crate::transports::manager::TransportManager;
use crate::transports::Transport;
use crate::types::channel::Channel as ChannelTrait;
use crate::types::channel::Event;
use crate::types::ice_transport::IceServer;
use crate::types::ice_transport::IceTransportInterface;

pub struct SwarmBuilder {
    key: Option<SecretKey>,
    ice_servers: Vec<IceServer>,
    external_address: Option<String>,
    dht_did: Option<Did>,
    dht_succ_max: u8,
    dht_storage: PersistenceStorage,
    session_manager: Option<SessionManager>,
    session_ttl: Option<Ttl>,
    callback: Option<CallbackFn>,
    /// support forward request to hidden services.
    hidden_service_port: Option<usize>
}

impl SwarmBuilder {
    pub fn new(ice_servers: &str, dht_storage: PersistenceStorage) -> Self {
        let ice_servers = ice_servers
            .split(';')
            .collect::<Vec<&str>>()
            .into_iter()
            .map(|s| IceServer::from_str(s).unwrap())
            .collect::<Vec<IceServer>>();
        SwarmBuilder {
            key: None,
            ice_servers,
            external_address: None,
            dht_did: None,
            dht_succ_max: 3,
            dht_storage,
            session_manager: None,
            session_ttl: None,
            callback: None,
            hidden_service_port: None
        }
    }

    pub fn dht_succ_max(mut self, succ_max: u8) -> Self {
        self.dht_succ_max = succ_max;
        self
    }

    pub fn callback(mut self, callback: CallbackFn) -> Self {
        self.callback = Some(callback);
        self
    }

    pub fn external_address(mut self, external_address: Option<String>) -> Self {
        self.external_address = external_address;
        self
    }

    pub fn key(mut self, key: SecretKey) -> Self {
        self.key = Some(key);
        self.dht_did = Some(key.address().into());
        self
    }

    pub fn session_manager(mut self, did: Did, session_manager: SessionManager) -> Self {
        self.session_manager = Some(session_manager);
        self.dht_did = Some(did);
        self
    }

    pub fn session_ttl(mut self, ttl: Ttl) -> Self {
        self.session_ttl = Some(ttl);
        self
    }

    pub fn build(self) -> Result<Swarm> {
        let session_manager = {
            if self.session_manager.is_some() {
                Ok(self.session_manager.unwrap())
            } else if self.key.is_some() {
                SessionManager::new_with_seckey(&self.key.unwrap(), self.session_ttl)
            } else {
                Err(Error::SwarmBuildFailed(
                    "Should set session_manager or key".into(),
                ))
            }
        }?;

        let dht_did = self
            .dht_did
            .ok_or_else(|| Error::SwarmBuildFailed("Should set session_manager or key".into()))?;

        let dht = PeerRing::new_with_storage(dht_did, self.dht_succ_max, self.dht_storage);

        Ok(Swarm {
            pending_transports: Arc::new(Mutex::new(vec![])),
            transports: MemStorage::new(),
            transport_event_channel: Channel::new(),
            ice_servers: self.ice_servers,
            external_address: self.external_address,
            dht: Arc::new(dht),
            session_manager,
            hidden_service_port: self.hidden_service_port,
            callback: self.callback,
        })
    }
}

pub struct Swarm {
    pub(crate) pending_transports: Arc<Mutex<Vec<Arc<Transport>>>>,
    pub(crate) transports: MemStorage<Did, Arc<Transport>>,
    pub(crate) ice_servers: Vec<IceServer>,
    pub(crate) transport_event_channel: Channel<Event>,
    pub(crate) external_address: Option<String>,
    dht: Arc<PeerRing>,
    pub callback: Option<CallbackFn>,
    /// support forward request to hidden services.
    pub hidden_service_port: Option<usize>,
    session_manager: SessionManager,
}

impl Swarm {
    pub fn did(&self) -> Did {
        self.dht.id
    }

    pub fn dht(&self) -> Arc<PeerRing> {
        self.dht.clone()
    }

    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    async fn load_message(
        &self,
        ev: Result<Option<Event>>,
    ) -> Result<Option<MessagePayload<Message>>> {
        let ev = ev?;

        match ev {
            Some(Event::DataChannelMessage(msg)) => {
                let payload = MessagePayload::from_encoded(&msg.try_into()?)?;
                Ok(Some(payload))
            }
            Some(Event::RegisterTransport((did, id))) => {
                // if transport is still pending
                if let Ok(Some(t)) = self.find_pending_transport(id) {
                    log::debug!("transport is inside pending list, mov to swarm transports");

                    self.register(did, t).await?;
                    self.pop_pending_transport(id)?;
                }
                match self.get_transport(did) {
                    Some(_) => {
                        let payload = MessagePayload::new_direct(
                            Message::JoinDHT(message::JoinDHT { id: did }),
                            &self.session_manager,
                            self.dht.id,
                        )?;
                        Ok(Some(payload))
                    }
                    None => Err(Error::SwarmMissTransport(did)),
                }
            }
            Some(Event::ConnectClosed((did, uuid))) => {
                if self.pop_pending_transport(uuid).is_ok() {
                    log::info!(
                        "[Swarm::ConnectClosed] Pending transport {:?} dropped",
                        uuid
                    );
                };

                if let Some(t) = self.get_transport(did) {
                    if t.id == uuid && self.remove_transport(did).is_some() {
                        log::info!("[Swarm::ConnectClosed] transport {:?} closed", uuid);
                        let payload = MessagePayload::new_direct(
                            Message::LeaveDHT(message::LeaveDHT { id: did }),
                            &self.session_manager,
                            self.dht.id,
                        )?;
                        return Ok(Some(payload));
                    }
                }
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// This method is required because web-sys components is not `Send`
    /// which means an async loop cannot running concurrency.
    pub async fn poll_message(&self) -> Option<MessagePayload<Message>> {
        let receiver = &self.transport_event_channel.receiver();
        let ev = Channel::recv(receiver).await;
        match self.load_message(ev).await {
            Ok(Some(msg)) => Some(msg),
            Ok(None) => None,
            Err(_) => None,
        }
    }

    pub async fn iter_messages<'a, 'b>(
        &'a self,
    ) -> impl Stream<Item = MessagePayload<Message>> + 'b
    where 'a: 'b {
        stream! {
            let receiver = &self.transport_event_channel.receiver();
            loop {
                let ev = Channel::recv(receiver).await;
                if let Ok(Some(msg)) = self.load_message(ev).await {
                    yield msg
                }
            }
        }
    }

    pub fn push_pending_transport(&self, transport: &Arc<Transport>) -> Result<()> {
        let mut pending = self
            .pending_transports
            .try_lock()
            .map_err(|_| Error::SwarmPendingTransTryLockFailed)?;
        pending.push(transport.to_owned());
        Ok(())
    }

    pub fn pop_pending_transport(&self, transport_id: uuid::Uuid) -> Result<()> {
        let mut pending = self
            .pending_transports
            .try_lock()
            .map_err(|_| Error::SwarmPendingTransTryLockFailed)?;
        let index = pending
            .iter()
            .position(|x| x.id.eq(&transport_id))
            .ok_or(Error::SwarmPendingTransNotFound)?;
        pending.remove(index);
        Ok(())
    }

    pub async fn pending_transports(&self) -> Result<Vec<Arc<Transport>>> {
        let pending = self
            .pending_transports
            .try_lock()
            .map_err(|_| Error::SwarmPendingTransTryLockFailed)?;
        Ok(pending.iter().cloned().collect::<Vec<_>>())
    }

    pub fn find_pending_transport(&self, id: uuid::Uuid) -> Result<Option<Arc<Transport>>> {
        let pending = self
            .pending_transports
            .try_lock()
            .map_err(|_| Error::SwarmPendingTransTryLockFailed)?;
        Ok(pending.iter().find(|x| x.id.eq(&id)).cloned())
    }
}

#[cfg_attr(feature = "wasm", async_trait(?Send))]
#[cfg_attr(not(feature = "wasm"), async_trait)]
impl<T> PayloadSender<T> for Swarm
where T: Clone + Serialize + DeserializeOwned + Send + Sync + 'static + fmt::Debug
{
    fn session_manager(&self) -> &SessionManager {
        Swarm::session_manager(self)
    }

    async fn do_send_payload(&self, did: Did, payload: MessagePayload<T>) -> Result<()> {
        #[cfg(test)]
        {
            println!("+++++++++++++++++++++++++++++++++");
            println!("node {:?}", self.dht.id);
            println!("Sent {:?}", payload.clone());
            println!("node {:?}", payload.relay.next_hop);
            println!("+++++++++++++++++++++++++++++++++");
        }
        let transport = self
            .get_and_check_transport(did)
            .await
            .ok_or(Error::SwarmMissDidInTable(did))?;
        log::trace!(
            "SENT {:?}, to node {:?} via transport {:?}",
            payload.clone(),
            payload.relay.next_hop,
            transport.id
        );
        let data: Vec<u8> = payload.encode()?.into();
        transport.wait_for_data_channel_open().await?;
        transport.send_message(data.as_slice()).await
    }
}

#[cfg(not(feature = "wasm"))]
#[cfg(test)]
pub mod tests {
    use tokio::time;
    use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;

    use super::*;
    use crate::ecc::SecretKey;
    #[cfg(not(feature = "dummy"))]
    use crate::transports::default::transport::tests::establish_connection;
    #[cfg(feature = "dummy")]
    use crate::transports::dummy::transport::tests::establish_connection;

    pub async fn new_swarm(key: SecretKey) -> Result<Swarm> {
        let stun = "stun://stun.l.google.com:19302";
        let storage =
            PersistenceStorage::new_with_path(PersistenceStorage::random_path("./tmp")).await?;
        SwarmBuilder::new(stun, storage).key(key).build()
    }

    #[tokio::test]
    async fn swarm_new_transport() -> Result<()> {
        let swarm = new_swarm(SecretKey::random()).await?;
        let transport = swarm.new_transport().await.unwrap();
        assert_eq!(
            transport.ice_connection_state().await.unwrap(),
            RTCIceConnectionState::New
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_swarm_register_and_get() -> Result<()> {
        let swarm1 = new_swarm(SecretKey::random()).await?;
        let swarm2 = new_swarm(SecretKey::random()).await?;

        assert!(swarm1.get_transport(swarm2.did()).is_none());
        assert!(swarm2.get_transport(swarm1.did()).is_none());

        let transport1 = swarm1.new_transport().await.unwrap();
        let transport2 = swarm2.new_transport().await.unwrap();

        establish_connection(&transport1, &transport2).await?;

        // Can register if connected
        swarm1.register(swarm2.did(), transport1.clone()).await?;
        swarm2.register(swarm1.did(), transport2.clone()).await?;

        // Check address transport pairs in transports
        let transport_1_to_2 = swarm1.get_transport(swarm2.did()).unwrap();
        let transport_2_to_1 = swarm2.get_transport(swarm1.did()).unwrap();

        assert!(Arc::ptr_eq(&transport_1_to_2, &transport1));
        assert!(Arc::ptr_eq(&transport_2_to_1, &transport2));

        Ok(())
    }

    #[tokio::test]
    async fn test_swarm_will_close_previous_transport() -> Result<()> {
        let swarm1 = new_swarm(SecretKey::random()).await?;
        let swarm2 = new_swarm(SecretKey::random()).await?;

        assert!(swarm1.get_transport(swarm2.did()).is_none());

        let transport0 = swarm1.new_transport().await.unwrap();
        let transport1 = swarm1.new_transport().await.unwrap();

        let transport_2_to_0 = swarm2.new_transport().await.unwrap();
        let transport_2_to_1 = swarm2.new_transport().await.unwrap();

        establish_connection(&transport0, &transport_2_to_0).await?;
        establish_connection(&transport1, &transport_2_to_1).await?;

        swarm1.register(swarm2.did(), transport0.clone()).await?;
        swarm1.register(swarm2.did(), transport1.clone()).await?;

        time::sleep(time::Duration::from_secs(3)).await;

        assert_eq!(
            transport0.ice_connection_state().await.unwrap(),
            RTCIceConnectionState::Closed
        );
        assert_eq!(
            transport_2_to_0.ice_connection_state().await.unwrap(),
            RTCIceConnectionState::Connected
        );
        // TODO: Find a way to maintain transports in another peer.

        assert_eq!(
            transport1.ice_connection_state().await.unwrap(),
            RTCIceConnectionState::Connected
        );
        assert_eq!(
            transport_2_to_1.ice_connection_state().await.unwrap(),
            RTCIceConnectionState::Connected
        );

        Ok(())
    }
}
