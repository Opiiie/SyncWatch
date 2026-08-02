use std::{
    collections::{HashMap, HashSet},
    net::{Ipv4Addr, SocketAddr},
    sync::Mutex,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{net::UdpSocket, sync::oneshot, time};

use crate::ws_server::{DiscoverableRoom, ServerState, WsServerInfo};

const DISCOVERY_PORT: u16 = 45_892;
const DISCOVERY_MULTICAST: Ipv4Addr = Ipv4Addr::new(239, 255, 77, 77);
const DISCOVERY_MAGIC: &str = "syncwatch-discovery";
const DISCOVERY_VERSION: u8 = 1;

#[derive(Default)]
pub struct DiscoveryController {
    cancel: Mutex<Option<oneshot::Sender<()>>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryQuery {
    protocol: String,
    version: u8,
    room_code: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredRoom {
    pub room_code: String,
    pub host_display_name: String,
    pub participant_count: usize,
    pub has_video: bool,
    pub address: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryResponse {
    protocol: String,
    version: u8,
    room_code: String,
    host_display_name: String,
    participant_count: usize,
    has_video: bool,
    port: u16,
}

impl DiscoveryController {
    pub async fn start(&self, state: ServerState, server: WsServerInfo) -> Result<(), String> {
        self.stop();
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT))
            .await
            .map_err(|_| "Не удалось включить поиск комнаты в локальной сети".to_owned())?;
        for (interface, _) in interface_broadcasts() {
            let _ = socket.join_multicast_v4(DISCOVERY_MULTICAST, interface);
        }
        let (cancel, mut cancelled) = oneshot::channel();
        *self.cancel.lock().map_err(|_| "Поиск комнат недоступен")? = Some(cancel);
        tokio::spawn(async move {
            let mut buffer = [0u8; 2_048];
            loop {
                tokio::select! {
                    _ = &mut cancelled => break,
                    received = socket.recv_from(&mut buffer) => {
                        let Ok((length, peer)) = received else { continue };
                        let Some(query) = parse_query(&buffer[..length]) else { continue };
                        let rooms = state.discoverable_rooms(query.room_code.as_deref()).await;
                        for room in rooms {
                            let response = response_for(room, server.port());
                            if let Ok(json) = serde_json::to_vec(&response) {
                                let _ = socket.send_to(&json, peer).await;
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }

    pub fn stop(&self) {
        if let Ok(mut current) = self.cancel.lock() {
            if let Some(cancel) = current.take() {
                let _ = cancel.send(());
            }
        }
    }
}

pub async fn discover_rooms(room_code: Option<String>) -> Result<Vec<DiscoveredRoom>, String> {
    let query = DiscoveryQuery {
        protocol: DISCOVERY_MAGIC.to_owned(),
        version: DISCOVERY_VERSION,
        room_code: room_code
            .map(|code| code.trim().to_uppercase())
            .filter(|code| !code.is_empty()),
    };
    let payload = serde_json::to_vec(&query).map_err(|_| "Не удалось начать поиск".to_owned())?;
    let mut targets = interface_broadcasts();
    targets.push((Ipv4Addr::LOCALHOST, Ipv4Addr::LOCALHOST));
    targets.sort_unstable();
    targets.dedup();

    let mut tasks = Vec::new();
    for (interface, broadcast) in targets {
        let payload = payload.clone();
        tasks.push(tokio::spawn(async move {
            discover_on_interface(interface, broadcast, payload).await
        }));
    }

    let mut found = HashMap::<String, DiscoveredRoom>::new();
    for task in tasks {
        if let Ok(rooms) = task.await {
            for room in rooms {
                match found.entry(room.room_code.clone()) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(room);
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry)
                        if prefers_discovery_address(&room, entry.get()) =>
                    {
                        entry.insert(room);
                    }
                    _ => {}
                }
            }
        }
    }
    let mut rooms = found.into_values().collect::<Vec<_>>();
    rooms.sort_by(|left, right| {
        left.host_display_name
            .to_lowercase()
            .cmp(&right.host_display_name.to_lowercase())
            .then_with(|| left.room_code.cmp(&right.room_code))
    });
    Ok(rooms)
}

fn prefers_discovery_address(candidate: &DiscoveredRoom, current: &DiscoveredRoom) -> bool {
    let candidate_loopback = candidate
        .address
        .parse::<SocketAddr>()
        .is_ok_and(|address| address.ip().is_loopback());
    let current_loopback = current
        .address
        .parse::<SocketAddr>()
        .is_ok_and(|address| address.ip().is_loopback());
    candidate_loopback && !current_loopback
}

async fn discover_on_interface(
    interface: Ipv4Addr,
    broadcast: Ipv4Addr,
    payload: Vec<u8>,
) -> Vec<DiscoveredRoom> {
    let Ok(socket) = UdpSocket::bind((interface, 0)).await else {
        return Vec::new();
    };
    let _ = socket.set_broadcast(true);
    let _ = socket.send_to(&payload, (broadcast, DISCOVERY_PORT)).await;
    let _ = socket
        .send_to(&payload, (DISCOVERY_MULTICAST, DISCOVERY_PORT))
        .await;
    if !interface.is_loopback() {
        let _ = socket
            .send_to(&payload, (Ipv4Addr::BROADCAST, DISCOVERY_PORT))
            .await;
    }

    let deadline = time::Instant::now() + Duration::from_millis(900);
    let mut buffer = [0u8; 2_048];
    let mut rooms = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(Ok((length, peer))) = time::timeout(remaining, socket.recv_from(&mut buffer)).await
        else {
            break;
        };
        if let Some(room) = parse_response(&buffer[..length], peer) {
            rooms.push(room);
        }
    }
    rooms
}

fn parse_query(bytes: &[u8]) -> Option<DiscoveryQuery> {
    let query = serde_json::from_slice::<DiscoveryQuery>(bytes).ok()?;
    (query.protocol == DISCOVERY_MAGIC && query.version == DISCOVERY_VERSION).then_some(query)
}

fn parse_response(bytes: &[u8], peer: SocketAddr) -> Option<DiscoveredRoom> {
    let response = serde_json::from_slice::<DiscoveryResponse>(bytes).ok()?;
    if response.protocol != DISCOVERY_MAGIC
        || response.version != DISCOVERY_VERSION
        || response.room_code.is_empty()
        || response.port == 0
    {
        return None;
    }
    Some(DiscoveredRoom {
        room_code: response.room_code,
        host_display_name: response.host_display_name,
        participant_count: response.participant_count,
        has_video: response.has_video,
        address: format!("{}:{}", peer.ip(), response.port),
    })
}

fn response_for(room: DiscoverableRoom, port: u16) -> DiscoveryResponse {
    DiscoveryResponse {
        protocol: DISCOVERY_MAGIC.to_owned(),
        version: DISCOVERY_VERSION,
        room_code: room.room_code,
        host_display_name: room.host_display_name,
        participant_count: room.participant_count,
        has_video: room.has_video,
        port,
    }
}

#[cfg(windows)]
fn interface_broadcasts() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    use std::{mem, ptr};
    use windows_sys::Win32::{
        Foundation::ERROR_BUFFER_OVERFLOW,
        NetworkManagement::{
            IpHelper::{
                GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
                GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
            },
            Ndis::IfOperStatusUp,
        },
        Networking::WinSock::{AF_INET, SOCKADDR_IN},
    };

    unsafe {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size = 0u32;
        if GetAdaptersAddresses(
            AF_INET as u32,
            flags,
            ptr::null(),
            ptr::null_mut(),
            &mut size,
        ) != ERROR_BUFFER_OVERFLOW
        {
            return Vec::new();
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(mem::size_of::<u64>())];
        let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        if GetAdaptersAddresses(AF_INET as u32, flags, ptr::null(), first, &mut size) != 0 {
            return Vec::new();
        }
        let mut result = HashSet::new();
        let mut adapter = first;
        while !adapter.is_null() {
            if (*adapter).OperStatus == IfOperStatusUp {
                let mut unicast = (*adapter).FirstUnicastAddress;
                while !unicast.is_null() {
                    let sockaddr = (*unicast).Address.lpSockaddr.cast::<SOCKADDR_IN>();
                    if !sockaddr.is_null() && (*sockaddr).sin_family == AF_INET {
                        let octets = (*sockaddr).sin_addr.S_un.S_un_b;
                        let ip = Ipv4Addr::new(octets.s_b1, octets.s_b2, octets.s_b3, octets.s_b4);
                        let prefix = (*unicast).OnLinkPrefixLength.min(32);
                        if !ip.is_unspecified() && !ip.is_loopback() && prefix > 0 {
                            let address = u32::from_be_bytes(ip.octets());
                            let mask = u32::MAX << (32 - prefix);
                            let broadcast = Ipv4Addr::from((address | !mask).to_be_bytes());
                            result.insert((ip, broadcast));
                        }
                    }
                    unicast = (*unicast).Next;
                }
            }
            adapter = (*adapter).Next;
        }
        result.into_iter().collect()
    }
}

#[cfg(not(windows))]
fn interface_broadcasts() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_response_uses_packet_source_address() {
        let response = DiscoveryResponse {
            protocol: DISCOVERY_MAGIC.to_owned(),
            version: DISCOVERY_VERSION,
            room_code: "ABC123".to_owned(),
            host_display_name: "Host".to_owned(),
            participant_count: 2,
            has_video: true,
            port: 34_567,
        };
        let bytes = serde_json::to_vec(&response).unwrap();
        let room = parse_response(&bytes, "26.10.20.30:45892".parse().unwrap()).unwrap();
        assert_eq!(room.address, "26.10.20.30:34567");
        assert_eq!(room.room_code, "ABC123");
    }

    #[test]
    fn discovery_rejects_other_protocols() {
        let query = br#"{"protocol":"something-else","version":1,"roomCode":null}"#;
        assert!(parse_query(query).is_none());
    }

    #[test]
    fn loopback_is_preferred_for_same_computer_discovery() {
        let network = DiscoveredRoom {
            room_code: "ABC123".to_owned(),
            host_display_name: "Host".to_owned(),
            participant_count: 1,
            has_video: true,
            address: "192.168.1.70:3412".to_owned(),
        };
        let loopback = DiscoveredRoom {
            address: "127.0.0.1:3412".to_owned(),
            ..network.clone()
        };
        assert!(prefers_discovery_address(&loopback, &network));
        assert!(!prefers_discovery_address(&network, &loopback));
    }
}
