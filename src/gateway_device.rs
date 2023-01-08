use anyhow::{bail, Result};
use igd::search_gateway;
use local_ip_address::local_ip;
use std::net::{IpAddr, SocketAddrV4};

pub const LOCAL_PORT_TCP: u16 = 46492;
pub const LOCAL_PORT_UDP: u16 = 46493;

pub fn open_any_port() -> Result<(u16, u16)> {
    let local_ip = match local_ip()? {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(_) => bail!("local ip is IPv6. not supported!"),
    };

    let gateway = search_gateway(Default::default())?;

    let tcp_port = gateway.add_any_port(
        igd::PortMappingProtocol::TCP,
        SocketAddrV4::new(local_ip, LOCAL_PORT_TCP),
        0,
        "TCP for peer protocol",
    )?;

    let udp_port = gateway.add_any_port(
        igd::PortMappingProtocol::UDP,
        SocketAddrV4::new(local_ip, LOCAL_PORT_UDP),
        0,
        "UDP for DHT",
    )?;

    Ok((tcp_port, udp_port))
}
