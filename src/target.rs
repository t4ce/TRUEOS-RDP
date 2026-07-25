use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct RemoteTarget {
    host: Arc<Mutex<String>>,
}

impl RemoteTarget {
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: Arc::new(Mutex::new(host.into())),
        }
    }

    pub fn host(&self) -> String {
        self.host
            .lock()
            .map(|host| host.clone())
            .unwrap_or_default()
    }

    pub fn set_host(&self, host: impl Into<String>) {
        if let Ok(mut current) = self.host.lock() {
            *current = host.into();
        }
    }

    pub fn resolve(&self, port: u16) -> io::Result<SocketAddr> {
        resolve_host(&self.host(), port)
    }
}

pub fn resolve_host(host: &str, port: u16) -> io::Result<SocketAddr> {
    let mut addresses = (host, port).to_socket_addrs()?;
    let first = addresses
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "host resolved no addresses"))?;
    if first.is_ipv4() {
        return Ok(first);
    }
    Ok(addresses.find(SocketAddr::is_ipv4).unwrap_or(first))
}

pub fn strip_port(target: &str) -> &str {
    let target = target.trim();
    if target.starts_with('[')
        && let Some(end) = target.find(']')
    {
        return &target[1..end];
    }
    if target.parse::<IpAddr>().is_ok() {
        return target;
    }
    if let Some((host, port)) = target.rsplit_once(':')
        && port.parse::<u16>().is_ok()
    {
        return host;
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_host_or_legacy_host_port() {
        assert_eq!(strip_port("192.168.178.94"), "192.168.178.94");
        assert_eq!(strip_port("192.168.178.94:100"), "192.168.178.94");
        assert_eq!(strip_port("trueos.eu:100"), "trueos.eu");
        assert_eq!(strip_port("[::1]:100"), "::1");
    }

    #[test]
    fn target_updates_are_shared() {
        let target = RemoteTarget::new("192.168.178.94");
        let worker = target.clone();
        target.set_host("trueos.eu");
        assert_eq!(worker.host(), "trueos.eu");
    }
}
