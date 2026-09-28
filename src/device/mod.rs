//! Native authenticated CoreDevice tunnel. Never creates or modifies pairing records.
mod offer;
mod pairing;

use anyhow::{Context, Result, bail};
use idevice::{
    IdeviceService, ReadWrite, RemoteXpcClient, RsdService,
    core_device::{
        ButtonState, IndigoHidClient, MainKeyboardService, UniversalHidServiceClient,
        build_start_video_parameters,
    },
    core_device_proxy::CoreDeviceProxy,
    remote_pairing::{RemotePairingClient, RpPairingSocket, connect_tls_psk_tunnel_native},
    rsd::RsdHandshake,
    tcp::{
        adapter::Adapter,
        handle::{AdapterHandle, UdpSocketHandle},
    },
    usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection},
    xpc::{Dictionary, XPCObject},
};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{net::TcpStream, time::timeout};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
type Xpc = RemoteXpcClient<Box<dyn ReadWrite>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnectionMode {
    #[default]
    Auto,
    Usb,
    Wifi,
}

#[derive(Debug, Default, Clone)]
pub struct DeviceOptions {
    pub connection: ConnectionMode,
    pub serial: Option<String>,
    pub address: Option<SocketAddr>,
    pub pairing_file: Option<PathBuf>,
}

pub struct DeviceSession {
    adapter: AdapterHandle,
    handshake: RsdHandshake,
    display: Xpc,
    session_id: uuid::Uuid,
    streaming: bool,
    // The remote pairing control socket must remain alive with its tunnel.
    _pairing: Option<RemotePairingClient<RpPairingSocket<TcpStream>>>,
}

#[derive(Debug, Clone, Copy)]
pub struct StreamInfo {
    pub local_ssrc: u32,
    pub remote_ssrc: u32,
    pub source_port: u16,
}
pub struct Datagram {
    pub data: Vec<u8>,
    pub source_port: u16,
}
pub struct VideoStream {
    socket: UdpSocketHandle,
    pub info: StreamInfo,
}

impl VideoStream {
    /// Ownership transfer avoids a second userspace packet copy.
    pub async fn recv(&self) -> Result<Datagram> {
        let packet = self.socket.recv().await.context("receive video datagram")?;
        Ok(Datagram {
            data: packet.data,
            source_port: packet.source_port,
        })
    }
    pub async fn send_rtcp(&self, packet: &[u8]) -> Result<()> {
        if self.info.source_port == 0 {
            bail!("device did not negotiate an RTCP source port");
        }
        self.socket
            .send_to(self.info.source_port, packet.to_vec())
            .await?;
        Ok(())
    }
}

impl DeviceSession {
    pub async fn connect(options: &DeviceOptions) -> Result<Self> {
        let usb = if options.connection != ConnectionMode::Wifi && options.address.is_none() {
            match timeout(CONNECT_TIMEOUT, connect_usb(options.serial.as_deref())).await {
                Ok(Ok(usb)) => Some(usb),
                Ok(Err(error)) if options.connection == ConnectionMode::Usb => return Err(error),
                Err(error) if options.connection == ConnectionMode::Usb => return Err(error.into()),
                _ => None,
            }
        } else {
            None
        };
        let (mut adapter, rsd_port, pairing) = if let Some((adapter, port)) = usb {
            tracing::info!(transport = "usb", "native tunnel connected");
            (adapter, port, None)
        } else {
            let path = pairing::find(options.pairing_file.as_deref(), options.serial.as_deref())?;
            let mut record = pairing::load(&path)?;
            let endpoints = match options.address {
                Some(address) => vec![address],
                None => discover().await?,
            };
            let mut connected = None;
            let mut last_error = None;
            for endpoint in endpoints {
                match timeout(CONNECT_TIMEOUT, connect_wifi(endpoint, &mut record)).await {
                    Ok(Ok(result)) => {
                        connected = Some(result);
                        break;
                    }
                    Ok(Err(error)) => last_error = Some(error),
                    Err(_) => {
                        last_error =
                            Some(anyhow::anyhow!("Wi-Fi pairing/tunnel handshake timed out"))
                    }
                }
            }
            let (adapter, port, rpc) = connected.ok_or_else(|| {
                last_error.unwrap_or_else(|| {
                    anyhow::anyhow!(
                        "no remote-pairing service found; verify iPhone is unlocked on the same LAN"
                    )
                })
            })?;
            tracing::info!(transport = "wifi", "native authenticated tunnel connected");
            (adapter, port, Some(rpc))
        };
        let handshake = timeout(CONNECT_TIMEOUT, async {
            let socket = adapter.connect(rsd_port).await?;
            RsdHandshake::new(socket).await
        })
        .await
        .context("RSD handshake timed out")??;
        let port = handshake
            .services
            .get("com.apple.coredevice.displayservice")
            .context("display service absent; iOS 27 and mounted developer image required")?
            .port;
        let display = timeout(CONNECT_TIMEOUT, async {
            let socket: Box<dyn ReadWrite> = Box::new(adapter.connect(port).await?);
            let mut client = RemoteXpcClient::new(socket).await?;
            client.do_handshake().await?;
            Ok::<_, idevice::IdeviceError>(client)
        })
        .await
        .context("display service connection timed out")??;
        Ok(Self {
            adapter,
            handshake,
            display,
            session_id: uuid::Uuid::new_v4(),
            streaming: false,
            _pairing: pairing,
        })
    }

    pub async fn start_video(&mut self) -> Result<VideoStream> {
        if self.streaming {
            bail!("this session already owns a stream");
        }
        let socket = self.adapter.bind_udp(0).await?;
        let local_ssrc = uuid::Uuid::new_v4().as_u128() as u32;
        let mut params = build_start_video_parameters(
            &self.adapter.host_ip().to_string(),
            socket.local_port(),
            &self.adapter.peer_ip().to_string(),
            50001,
            offer::build(local_ssrc)?,
            140,
            1,
            self.session_id,
        );
        // Match the working controller: the daemon assigns its own source port.
        params.shift_remove("senderPort");
        params.insert("timeout".into(), XPCObject::UInt64(20));
        let answer = timeout(
            CONNECT_TIMEOUT,
            invoke(
                &mut self.display,
                "startmediastream",
                "mediastreamstart",
                params,
            ),
        )
        .await
        .context("video negotiation timed out")??;
        self.streaming = true;
        let config = answer
            .as_dictionary()
            .and_then(|d| d.get("connection"))
            .and_then(plist::Value::as_dictionary)
            .and_then(|d| d.get("streamConfig"))
            .and_then(plist::Value::as_dictionary)
            .context("video answer has no stream configuration")?;
        let number = |key: &str| config.get(key).and_then(plist::Value::as_unsigned_integer);
        let info = StreamInfo {
            local_ssrc: number("RemoteSSRC").unwrap_or(local_ssrc as u64) as u32,
            remote_ssrc: number("LocalSSRC").unwrap_or(0) as u32,
            source_port: number("SourcePort").unwrap_or(0) as u16,
        };
        tracing::info!(
            local_ssrc = info.local_ssrc,
            remote_ssrc = info.remote_ssrc,
            rtcp_port = info.source_port,
            "video stream negotiated"
        );
        Ok(VideoStream { socket, info })
    }

    pub async fn open_input(&mut self) -> Result<HidChannels> {
        let mut universal = timeout(
            CONNECT_TIMEOUT,
            UniversalHidServiceClient::connect_rsd(&mut self.adapter, &mut self.handshake),
        )
        .await??;
        let indigo = timeout(
            CONNECT_TIMEOUT,
            IndigoHidClient::connect_rsd(&mut self.adapter, &mut self.handshake),
        )
        .await??;
        let keyboard = timeout(CONNECT_TIMEOUT, universal.create_main_keyboard()).await??;
        Ok(HidChannels {
            universal,
            indigo,
            keyboard,
            touch: None,
            home_pressed: false,
        })
    }

    pub async fn stop(&mut self) -> Result<()> {
        let stopped = if self.streaming {
            let mut params = Dictionary::new();
            let mut id = Dictionary::new();
            id.insert("uuid".into(), XPCObject::Uuid(self.session_id));
            params.insert(
                "avcMediaStreamOptionClientSessionID".into(),
                XPCObject::Dictionary(id),
            );
            timeout(
                Duration::from_secs(3),
                invoke(
                    &mut self.display,
                    "stopmediastream",
                    "mediastreamstop",
                    params,
                ),
            )
            .await
            .context("stopping video timed out")
            .and_then(|result| match result {
                Ok(_) => Ok(()),
                // The daemon closes its XPC channel immediately after a successful stop.
                Err(error) if error.downcast_ref::<idevice::IdeviceError>().is_some_and(|e| {
                    matches!(e, idevice::IdeviceError::Socket(io) if matches!(io.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe))
                }) => Ok(()),
                Err(error) => Err(error),
            })
        } else {
            Ok(())
        };
        self.streaming = false;
        self.adapter.close().await?;
        stopped
    }
}

pub struct HidChannels {
    universal: UniversalHidServiceClient<Box<dyn ReadWrite>>,
    indigo: IndigoHidClient<Box<dyn ReadWrite>>,
    keyboard: MainKeyboardService,
    touch: Option<[u8; 58]>,
    home_pressed: bool,
}
impl HidChannels {
    pub async fn send(&mut self, event: &crate::input::HidEvent) -> Result<()> {
        use crate::input::HidEvent;
        match event {
            HidEvent::Touch { report, .. } => {
                // Record before awaiting: a canceled write may have reached the device.
                if report[3] == 0xc2 {
                    self.touch = Some(*report);
                }
                self.universal.send_report(257, report.to_vec()).await?;
                if report[3] != 0xc2 {
                    self.touch = None;
                }
            }
            HidEvent::Keyboard(report) => {
                self.universal
                    .send_report(0x1_0000_2001, report.to_vec())
                    .await?
            }
            HidEvent::Home { pressed } => {
                if *pressed {
                    self.home_pressed = true;
                }
                self.indigo
                    .send_button(
                        12,
                        64,
                        if *pressed {
                            ButtonState::Down
                        } else {
                            ButtonState::Up
                        },
                    )
                    .await?;
                if !pressed {
                    self.home_pressed = false;
                }
            }
        }
        Ok(())
    }
    pub async fn close(&mut self) -> Result<()> {
        let mut first_error = None;
        if let Some(mut report) = self.touch.take() {
            report[3] = 2;
            if let Err(error) = timeout(
                Duration::from_millis(650),
                self.universal.send_report(257, report.to_vec()),
            )
            .await
            .context("touch release timed out")
            .and_then(|r| r.map_err(Into::into))
            {
                first_error = Some(error);
            }
        }
        if self.home_pressed {
            self.home_pressed = false;
            if let Err(error) = timeout(
                Duration::from_millis(650),
                self.indigo.send_button(12, 64, ButtonState::Up),
            )
            .await
            .context("Home release timed out")
            .and_then(|r| r.map_err(Into::into))
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Err(error) = timeout(
            Duration::from_millis(1500),
            self.universal.remove_main_keyboard(&mut self.keyboard),
        )
        .await
        .context("keyboard removal timed out")
        .and_then(|r| r.map_err(Into::into))
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
    }
}

async fn invoke(
    client: &mut Xpc,
    feature: &str,
    action: &str,
    input: Dictionary,
) -> Result<plist::Value> {
    let mut request = Dictionary::new();
    request.insert(
        "CoreDevice.CoreDeviceDDIProtocolVersion".into(),
        XPCObject::Int64(2),
    );
    let mut version = Dictionary::new();
    version.insert(
        "components".into(),
        XPCObject::Array(vec![XPCObject::UInt64(629), XPCObject::UInt64(3)]),
    );
    version.insert("originalComponentsCount".into(), XPCObject::Int64(2));
    version.insert("stringValue".into(), XPCObject::String("629.3".into()));
    request.insert(
        "CoreDevice.coreDeviceVersion".into(),
        XPCObject::Dictionary(version),
    );
    request.insert(
        "CoreDevice.action".into(),
        XPCObject::Dictionary(Dictionary::new()),
    );
    request.insert(
        "CoreDevice.featureIdentifier".into(),
        XPCObject::String(format!("com.apple.coredevice.feature.{feature}")),
    );
    request.insert(
        "CoreDevice.actionIdentifier".into(),
        XPCObject::String(format!("com.apple.coredevice.action.{action}")),
    );
    request.insert(
        "CoreDevice.deviceIdentifier".into(),
        XPCObject::String(uuid::Uuid::new_v4().to_string()),
    );
    request.insert(
        "CoreDevice.invocationIdentifier".into(),
        XPCObject::String(uuid::Uuid::new_v4().to_string()),
    );
    request.insert("CoreDevice.input".into(), XPCObject::Dictionary(input));
    client.send_object(request, true).await?;
    let response = client.recv().await?;
    let dict = response
        .as_dictionary()
        .context("invalid CoreDevice reply")?;
    if let Some(output) = dict.get("CoreDevice.output") {
        return Ok(output.clone());
    }
    // Device errors can embed names and identifiers; retain only the action here.
    bail!("CoreDevice rejected {action} (no output returned)")
}

async fn connect_usb(serial: Option<&str>) -> Result<(AdapterHandle, u16)> {
    let mut mux = UsbmuxdConnection::default().await?;
    let mut devices = mux.get_devices().await?.into_iter().filter(|d| {
        d.connection_type == Connection::Usb
            && serial.is_none_or(|s| s.replace('-', "") == d.udid.replace('-', ""))
    });
    let device = devices.next().context("no matching USB iPhone connected")?;
    if devices.next().is_some() {
        bail!("several USB iPhones connected; choose --serial");
    }
    let provider = device.to_provider(UsbmuxdAddr::default(), "iphone-mirror-rs");
    let proxy = CoreDeviceProxy::connect(&provider).await?;
    let port = proxy.tunnel_info().server_rsd_port;
    Ok((proxy.create_software_tunnel()?.to_async_handle(), port))
}

async fn connect_wifi(
    address: SocketAddr,
    record: &mut idevice::remote_pairing::RpPairingFile,
) -> Result<(
    AdapterHandle,
    u16,
    RemotePairingClient<RpPairingSocket<TcpStream>>,
)> {
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    let mut rpc = RemotePairingClient::new(RpPairingSocket::new(stream), "iphone-mirror-rs");
    rpc.attempt_pair_verify().await?;
    rpc.validate_pairing(record)
        .await
        .context("saved CoreDevice pairing was rejected")?;
    let port = rpc.create_tcp_listener().await?;
    let mut tunnel_address = address;
    tunnel_address.set_port(port);
    let stream = TcpStream::connect(tunnel_address).await?;
    stream.set_nodelay(true)?;
    let tunnel = connect_tls_psk_tunnel_native(stream, rpc.encryption_key()).await?;
    let rsd_port = tunnel.info.server_rsd_port;
    let host = tunnel.info.client_address.parse()?;
    let peer = tunnel.info.server_address.parse()?;
    let mtu = tunnel.info.mtu as usize;
    let mut adapter = Adapter::new(Box::new(tunnel.into_inner()), host, peer);
    adapter.set_mss(mtu.saturating_sub(60));
    Ok((adapter.to_async_handle(), rsd_port, rpc))
}

async fn discover() -> Result<Vec<SocketAddr>> {
    let mdns = mdns_sd::ServiceDaemon::new()?;
    let receiver = mdns.browse("_remotepairing._tcp.local.")?;
    let mut addresses = Vec::new();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Ok(Ok(event)) = tokio::time::timeout_at(end, receiver.recv_async()).await {
        if let mdns_sd::ServiceEvent::ServiceResolved(service) = event {
            for ip in service.get_addresses() {
                let address = match ip {
                    mdns_sd::ScopedIp::V4(ip) => {
                        SocketAddr::new((*ip.addr()).into(), service.get_port())
                    }
                    mdns_sd::ScopedIp::V6(ip) => SocketAddr::V6(std::net::SocketAddrV6::new(
                        *ip.addr(),
                        service.get_port(),
                        0,
                        ip.scope_id().index,
                    )),
                    _ => continue,
                };
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
        }
    }
    let _ = mdns.stop_browse("_remotepairing._tcp.local.");
    let _ = mdns.shutdown();
    addresses.sort_by_key(|a| a.is_ipv6());
    Ok(addresses)
}
