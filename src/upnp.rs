//! Automatic port forwarding with UPnP, so a game hosted on a home computer
//! can be joined over the internet.
//!
//! The router (an "Internet Gateway Device") is found with SSDP, then asked
//! over HTTP/SOAP to forward the game's TCP port to this computer
//! (AddPortMapping) and for its public address (GetExternalIPAddress).  If
//! it also offers IPv6 firewall control, a pinhole is opened for this
//! computer's IPv6 address.  Everything is removed again when the game ends.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

const SSDP_ADDR: &str = "239.255.255.250:1900";
const HTTP_TIMEOUT: Duration = Duration::from_secs(4);

/// A router that can forward ports.
#[derive(Clone, Debug)]
pub struct Gateway {
    /// http://host:port/path of the WANIPConnection / WANPPPConnection service
    control_url: String,
    service: String,
    /// the IPv6 firewall service, if the router has one
    v6_control_url: Option<String>,
    v6_service: Option<String>,
    /// this computer's address on the router's network
    pub local_ip: Ipv4Addr,
}

/// What the router did for us; `remove` undoes it.
#[derive(Debug)]
pub struct Mapping {
    pub gateway: Gateway,
    pub external_port: u16,
    pub external_ip: Option<Ipv4Addr>,
    pinhole: Option<String>,
}

// ---------------------------------------------------------------------------
// addresses
// ---------------------------------------------------------------------------

/// Not reachable from the internet: private, shared (carrier-grade NAT),
/// link-local, loopback or unspecified.
pub fn is_private_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || (o[0] == 100 && (o[1] & 0xc0) == 64)
}

/// The carrier-grade NAT range (100.64.0.0/10).
pub fn is_cgnat_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

/// This computer's address on the local network (the one it would use to
/// reach the internet).  No packet is sent.
pub fn lan_ipv4() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:53").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_loopback() => Some(ip),
        _ => None,
    }
}

/// This computer's global IPv6 address, if it has one.  No packet is sent.
pub fn global_ipv6() -> Option<Ipv6Addr> {
    let s = UdpSocket::bind("[::]:0").ok()?;
    s.connect("[2001:4860:4860::8888]:53").ok()?;
    match s.local_addr().ok()?.ip() {
        // global unicast is 2000::/3
        IpAddr::V6(ip) if (ip.segments()[0] & 0xe000) == 0x2000 => Some(ip),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// discovery
// ---------------------------------------------------------------------------

/// Find the router (waits up to `wait`).
pub fn discover(wait: Duration) -> Result<Gateway, String> {
    if let Ok(loc) = std::env::var("SNOWBALLZ_UPNP_LOCATION") {
        return gateway_from_location(&loc);
    }
    let target = std::env::var("SNOWBALLZ_SSDP_ADDR").unwrap_or_else(|_| SSDP_ADDR.to_string());
    discover_at(&target, wait)
}

fn discover_at(target: &str, wait: Duration) -> Result<Gateway, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    let _ = sock.set_read_timeout(Some(Duration::from_millis(150)));
    let _ = sock.set_multicast_ttl_v4(2);
    let search = |sock: &UdpSocket| {
        for st in [
            "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
            "urn:schemas-upnp-org:device:InternetGatewayDevice:2",
            "urn:schemas-upnp-org:service:WANIPConnection:1",
            "urn:schemas-upnp-org:service:WANPPPConnection:1",
        ] {
            let msg = format!(
                "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nST: {}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\n\r\n",
                st
            );
            let _ = sock.send_to(msg.as_bytes(), target);
        }
    };
    search(&sock);
    let start = Instant::now();
    let mut resent = false;
    let mut tried: Vec<String> = Vec::new();
    let mut last_err = "no router answered (UPnP may be turned off in its settings)".to_string();
    let mut buf = [0u8; 2048];
    while start.elapsed() < wait {
        if !resent && start.elapsed() > wait / 2 {
            resent = true;
            search(&sock); // UDP can get lost
        }
        let Ok((n, _from)) = sock.recv_from(&mut buf) else { continue };
        let text = String::from_utf8_lossy(&buf[..n]);
        let Some(loc) = header(&text, "location") else { continue };
        if tried.contains(&loc) {
            continue;
        }
        tried.push(loc.clone());
        match gateway_from_location(&loc) {
            Ok(g) => return Ok(g),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// A header's value (case-insensitive name).
fn header(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim().eq_ignore_ascii_case(name) {
            Some(v.trim().to_string())
        } else {
            None
        }
    })
}

/// Read the router's description and find its port forwarding service.
pub fn gateway_from_location(location: &str) -> Result<Gateway, String> {
    let (host, port, path) = split_url(location)?;
    let (status, body, local) = http(&host, port, &format!("GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n", path, host, port))?;
    if status != 200 {
        return Err(format!("the router's description gave HTTP {}", status));
    }
    let base = tag(&body, "URLBase").map(|s| s.trim().to_string()).unwrap_or_else(|| format!("http://{}:{}", host, port));
    let mut ip = None;
    let mut v6 = None;
    for svc in blocks(&body, "service") {
        let (Some(kind), Some(ctl)) = (tag(svc, "serviceType"), tag(svc, "controlURL")) else { continue };
        let kind = kind.trim().to_string();
        let url = join_url(&base, ctl.trim());
        if ip.is_none() && (kind.contains(":WANIPConnection:") || kind.contains(":WANPPPConnection:")) {
            ip = Some((url, kind));
        } else if v6.is_none() && kind.contains(":WANIPv6FirewallControl:") {
            v6 = Some((url, kind));
        }
    }
    let (control_url, service) = ip.ok_or("the router does not offer port forwarding (no WANIPConnection)")?;
    let local_ip = match local {
        IpAddr::V4(v) => v,
        _ => lan_ipv4().ok_or("no IPv4 address on this computer")?,
    };
    Ok(Gateway { control_url, service, v6_control_url: v6.as_ref().map(|v| v.0.clone()), v6_service: v6.map(|v| v.1), local_ip })
}

// ---------------------------------------------------------------------------
// port forwarding
// ---------------------------------------------------------------------------

impl Gateway {
    /// Forward TCP `port` (or, if another computer already uses it, one of
    /// the next few external ports) to this computer.
    pub fn add_mapping(&self, port: u16, description: &str) -> Result<u16, String> {
        let mut last = String::new();
        for external in port..port.saturating_add(5) {
            let mut conflict = false;
            // a one-day lease first (so a crashed game's forwarding goes
            // away by itself); some old routers only take permanent ones
            for lease in ["86400", "0"] {
                let r = self.soap(
                    "AddPortMapping",
                    &[
                        ("NewRemoteHost", String::new()),
                        ("NewExternalPort", external.to_string()),
                        ("NewProtocol", "TCP".into()),
                        ("NewInternalPort", port.to_string()),
                        ("NewInternalClient", self.local_ip.to_string()),
                        ("NewEnabled", "1".into()),
                        ("NewPortMappingDescription", description.into()),
                        ("NewLeaseDuration", lease.into()),
                    ],
                );
                match r {
                    Ok(_) => return Ok(external),
                    Err(e) => {
                        conflict = e.starts_with("718");
                        last = e;
                        if conflict {
                            break;
                        }
                    }
                }
            }
            if !conflict {
                break; // refused for another reason: other ports won't help
            }
            // 718: another computer uses this port: try the next one
        }
        Err(last)
    }

    pub fn remove_mapping(&self, external: u16) -> Result<(), String> {
        self.soap(
            "DeletePortMapping",
            &[("NewRemoteHost", String::new()), ("NewExternalPort", external.to_string()), ("NewProtocol", "TCP".into())],
        )
        .map(|_| ())
    }

    pub fn external_ip(&self) -> Result<Ipv4Addr, String> {
        let body = self.soap("GetExternalIPAddress", &[])?;
        let ip = tag(&body, "NewExternalIPAddress").ok_or("no address in the router's answer")?;
        ip.trim().parse().map_err(|_| format!("the router gave a strange address: {}", ip.trim()))
    }

    /// Let other computers reach `addr`:`port` over IPv6 (if the router
    /// has an IPv6 firewall that can be opened this way).
    pub fn open_v6_pinhole(&self, addr: Ipv6Addr, port: u16) -> Result<String, String> {
        let (Some(url), Some(svc)) = (&self.v6_control_url, &self.v6_service) else {
            return Err("the router has no IPv6 firewall control".into());
        };
        let body = soap_call(
            url,
            svc,
            "AddPinhole",
            &[
                ("RemoteHost", String::new()),
                ("RemotePort", "0".into()),
                ("InternalClient", addr.to_string()),
                ("InternalPort", port.to_string()),
                ("Protocol", "6".into()),
                ("LeaseTime", "86400".into()),
            ],
        )?;
        Ok(tag(&body, "UniqueID").map(|s| s.trim().to_string()).unwrap_or_default())
    }

    pub fn close_v6_pinhole(&self, id: &str) {
        if let (Some(url), Some(svc)) = (&self.v6_control_url, &self.v6_service) {
            let _ = soap_call(url, svc, "DeletePinhole", &[("UniqueID", id.to_string())]);
        }
    }

    fn soap(&self, action: &str, args: &[(&str, String)]) -> Result<String, String> {
        soap_call(&self.control_url, &self.service, action, args)
    }
}

/// Find the router, forward `port` and ask for the public address.
pub fn open(port: u16, description: &str, wait: Duration) -> Result<Mapping, String> {
    let gateway = discover(wait)?;
    let external_port = gateway.add_mapping(port, description).map_err(|e| format!("the router refused to forward the port: {}", e))?;
    let external_ip = gateway.external_ip().ok();
    let pinhole = global_ipv6().and_then(|v6| gateway.open_v6_pinhole(v6, port).ok());
    Ok(Mapping { gateway, external_port, external_ip, pinhole })
}

impl Mapping {
    /// Undo the forwarding (when the game ends).
    pub fn remove(&self) {
        let _ = self.gateway.remove_mapping(self.external_port);
        if let Some(id) = &self.pinhole {
            self.gateway.close_v6_pinhole(id);
        }
    }
}

// ---------------------------------------------------------------------------
// tiny HTTP / SOAP / XML helpers
// ---------------------------------------------------------------------------

fn soap_call(url: &str, service: &str, action: &str, args: &[(&str, String)]) -> Result<String, String> {
    let (host, port, path) = split_url(url)?;
    let mut inner = String::new();
    for (k, v) in args {
        inner.push_str(&format!("<{k}>{}</{k}>", xml_escape(v)));
    }
    let body = format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
         <u:{action} xmlns:u=\"{service}\">{inner}</u:{action}></s:Body></s:Envelope>\r\n"
    );
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\n\
         SOAPAction: \"{service}#{action}\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (status, resp, _) = http(&host, port, &req)?;
    if status == 200 {
        Ok(resp)
    } else {
        let code = tag(&resp, "errorCode").map(|s| s.trim().to_string()).unwrap_or_else(|| format!("HTTP {}", status));
        let desc = tag(&resp, "errorDescription").map(|s| s.trim().to_string()).unwrap_or_default();
        Err(format!("{} {}", code, desc).trim().to_string())
    }
}

/// One HTTP request; returns (status, body, this computer's address).
fn http(host: &str, port: u16, request: &str) -> Result<(u16, String, IpAddr), String> {
    let addr: SocketAddr = (host, port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("bad router address")?;
    let mut s = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT).map_err(|e| format!("router: {}", e))?;
    let _ = s.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = s.set_write_timeout(Some(HTTP_TIMEOUT));
    let local = s.local_addr().map_err(|e| e.to_string())?.ip();
    s.write_all(request.as_bytes()).map_err(|e| format!("router: {}", e))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > 1 << 20 {
                    break;
                }
                if let Some(done) = complete_response(&raw) {
                    raw.truncate(done);
                    break;
                }
            }
            Err(_) if !raw.is_empty() => break, // some routers keep the connection open
            Err(e) => return Err(format!("router: {}", e)),
        }
    }
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("bad answer from the router")?;
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("bad answer from the router")?;
    let body = if header(head, "transfer-encoding").map(|v| v.eq_ignore_ascii_case("chunked")).unwrap_or(false) {
        dechunk(body)
    } else {
        body.to_string()
    };
    Ok((status, body, local))
}

/// The length of a complete response with Content-Length, if it is complete.
fn complete_response(raw: &[u8]) -> Option<usize> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&raw[..end]);
    let len: usize = header(&head, "content-length")?.parse().ok()?;
    (raw.len() >= end + len).then_some(end + len)
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        if n == 0 || after.len() < n {
            out.push_str(&after[..n.min(after.len())]);
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

/// http://host[:port][/path] -> (host, port, path)
fn split_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| format!("not an http address: {}", url))?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let (host, port) = if let Some(h) = hostport.strip_prefix('[') {
        let (h, p) = h.split_once(']').ok_or("bad address")?;
        (h.to_string(), p.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(80))
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().map_err(|_| "bad port")?),
            None => (hostport.to_string(), 80),
        }
    };
    Ok((host, port, path))
}

fn join_url(base: &str, url: &str) -> String {
    if url.starts_with("http://") {
        return url.to_string();
    }
    // scheme://host:port of the base
    let root = match base.strip_prefix("http://").and_then(|r| r.find('/').map(|i| &base[..7 + i])) {
        Some(r) => r.to_string(),
        None => base.trim_end_matches('/').to_string(),
    };
    if url.starts_with('/') {
        format!("{}{}", root, url)
    } else {
        format!("{}/{}", root, url)
    }
}

/// The text inside the first <name>...</name> (any namespace prefix).
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    blocks(xml, name).into_iter().next()
}

/// The texts inside every <name>...</name> (also <x:name>).
fn blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = xml[pos..].find('<') {
        let start = pos + i;
        let Some(end_rel) = xml[start..].find('>') else { break };
        let open = &xml[start + 1..start + end_rel];
        pos = start + end_rel + 1;
        let tname = open.split_whitespace().next().unwrap_or("");
        let local = tname.rsplit(':').next().unwrap_or("");
        if local != name || open.starts_with('/') || open.ends_with('/') {
            continue;
        }
        // the matching close tag (same prefix)
        let close = format!("</{}>", tname);
        if let Some(c) = xml[pos..].find(&close) {
            out.push(&xml[pos..pos + c]);
            pos += c + close.len();
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// tests: a fake router
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A pretend router: serves a description and answers SOAP calls,
    /// recording the actions.  `taken` external ports answer error 718.
    fn fake_router(taken: Vec<u16>, chunked: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                // read the head and the body (Content-Length)
                loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                    if complete_request(&req) {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&req).into_owned();
                let (status, body) = if text.starts_with("GET /rootDesc.xml") {
                    (200, DESC.to_string())
                } else if let Some(action) = header(&text, "soapaction") {
                    let action = action.trim_matches('"').rsplit('#').next().unwrap().to_string();
                    log2.lock().unwrap().push(format!("{} {}", action, tag(&text, "NewExternalPort").unwrap_or("")));
                    match action.as_str() {
                        "AddPortMapping" if taken.iter().any(|p| Some(p.to_string().as_str()) == tag(&text, "NewExternalPort")) => {
                            (500, "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>718</errorCode><errorDescription>ConflictInMappingEntry</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>".to_string())
                        }
                        "GetExternalIPAddress" => (200, "<s:Envelope><s:Body><u:GetExternalIPAddressResponse xmlns:u=\"x\"><NewExternalIPAddress>203.0.113.7</NewExternalIPAddress></u:GetExternalIPAddressResponse></s:Body></s:Envelope>".to_string()),
                        _ => (200, "<s:Envelope><s:Body/></s:Envelope>".to_string()),
                    }
                } else {
                    (404, String::new())
                };
                let resp = if chunked {
                    let half = body.len() / 2;
                    format!(
                        "HTTP/1.1 {} X\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
                        status,
                        half,
                        &body[..half],
                        body.len() - half,
                        &body[half..]
                    )
                } else {
                    format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\n\r\n{}", status, body.len(), body)
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });
        (format!("http://127.0.0.1:{}/rootDesc.xml", port), log)
    }

    fn complete_request(raw: &[u8]) -> bool {
        let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else { return false };
        let head = String::from_utf8_lossy(&raw[..end]);
        let len: usize = header(&head, "content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        raw.len() >= end + 4 + len
    }

    const DESC: &str = r#"<?xml version="1.0"?>
<root xmlns="urn:schemas-upnp-org:device-1-0"><device>
<deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>
<deviceList><device><deviceType>urn:schemas-upnp-org:device:WANDevice:1</deviceType>
<serviceList><service><serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType><controlURL>/ctl/CmnIfCfg</controlURL></service></serviceList>
<deviceList><device><deviceType>urn:schemas-upnp-org:device:WANConnectionDevice:1</deviceType>
<serviceList>
<service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType><controlURL>/ctl/IPConn</controlURL></service>
<service><serviceType>urn:schemas-upnp-org:service:WANIPv6FirewallControl:1</serviceType><controlURL>ctl/IP6FCtl</controlURL></service>
</serviceList></device></deviceList></device></deviceList></device></root>"#;

    #[test]
    fn forwards_a_port_with_a_fake_router() {
        for chunked in [false, true] {
            let (loc, log) = fake_router(vec![5555], chunked);
            let g = gateway_from_location(&loc).unwrap();
            assert!(g.control_url.ends_with("/ctl/IPConn"), "{}", g.control_url);
            assert!(g.v6_control_url.as_deref().unwrap().ends_with("/ctl/IP6FCtl"));
            assert_eq!(g.local_ip, Ipv4Addr::LOCALHOST);
            // 5555 is taken by another computer: the next port is used
            assert_eq!(g.add_mapping(5555, "Snowballz").unwrap(), 5556);
            assert_eq!(g.external_ip().unwrap(), Ipv4Addr::new(203, 0, 113, 7));
            g.remove_mapping(5556).unwrap();
            let log = log.lock().unwrap().clone();
            assert_eq!(log, ["AddPortMapping 5555", "AddPortMapping 5556", "GetExternalIPAddress ", "DeletePortMapping 5556"]);
        }
    }

    #[test]
    fn finds_the_router_with_ssdp() {
        let (loc, _) = fake_router(vec![], false);
        // a pretend SSDP responder on localhost
        let ssdp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let target = ssdp.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            if let Ok((n, from)) = ssdp.recv_from(&mut buf) {
                assert!(String::from_utf8_lossy(&buf[..n]).starts_with("M-SEARCH"));
                let answer = format!("HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=120\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nLOCATION: {}\r\n\r\n", loc);
                let _ = ssdp.send_to(answer.as_bytes(), from);
            }
        });
        let g = discover_at(&target, Duration::from_secs(2)).unwrap();
        assert!(g.control_url.ends_with("/ctl/IPConn"));
    }

    #[test]
    fn address_helpers() {
        assert!(is_private_v4("192.168.1.20".parse().unwrap()));
        assert!(is_private_v4("100.72.1.1".parse().unwrap()));
        assert!(is_cgnat_v4("100.127.255.1".parse().unwrap()));
        assert!(!is_private_v4("203.0.113.7".parse().unwrap()));
        assert_eq!(split_url("http://[fe80::1]:5000/x").unwrap(), ("fe80::1".to_string(), 5000, "/x".to_string()));
        assert_eq!(join_url("http://10.0.0.1:5000/desc.xml", "ctl"), "http://10.0.0.1:5000/ctl");
        assert_eq!(dechunk("4\r\nabcd\r\n2\r\nef\r\n0\r\n\r\n"), "abcdef");
    }
}
