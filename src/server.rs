extern crate alloc;

use alloc::vec::Vec;
use alloc::{format, vec};

use defmt::warn;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::socket::tcp::State;
use smoltcp::time::Instant;
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address};

use crate::get_lps;
use crate::ncm_netif::{EthRingBuffers, StmPhy};
use defmt::info;

use crate::http::{
    gen_http_header, CallbackBt, HttpCallback, HttpContentType, HttpEncodingType, HttpError,
    HttpRequest, Httpserver, HTTP_404_RESPONSE,
};

const TESTWEBSITE: &[u8] = include_bytes!("../static/mockup_mini.html.gz");

struct HttpGetHandle {
    data: &'static [u8],
}

impl HttpCallback for HttpGetHandle {
    fn handle_request(&self, request: &HttpRequest) -> Vec<u8> {
        info!("{}", request);
        match request.path.as_str() {
            "/" | "/index.html" => {
                let mut buf: Vec<u8> = gen_http_header(
                    Some(self.data),
                    HttpContentType::Text,
                    Some(HttpEncodingType::Gzip),
                );
                buf.extend_from_slice(TESTWEBSITE);
                buf
            }
            "/lps" => {
                let lps = &format!("{}", get_lps()).into_bytes();
                let mut buf = gen_http_header(Some(lps), HttpContentType::Data, None);
                buf.extend_from_slice(lps);
                buf
            }
            _ => HTTP_404_RESPONSE.into(),
        }
    }
}

const HTTPGETHANDLE: HttpGetHandle = HttpGetHandle { data: TESTWEBSITE };
const HTTPPOSTHANDLE: HttpPostHandle = HttpPostHandle;

const RINGBUFSIZE: usize = 128;

struct HttpPostHandle;

impl HttpCallback for HttpPostHandle {
    fn handle_request(&self, request: &HttpRequest) -> Vec<u8> {
        info!("{}", request);
        match request.path.as_str() {
            "/rgb" => gen_http_header(None, HttpContentType::Text, None),
            _ => HTTP_404_RESPONSE.into(),
        }
    }
}

pub struct TcpServer<'a> {
    device: StmPhy,
    iface: Interface,
    sockets: SocketSet<'a>,
    tcp1_handle: SocketHandle,
    rxbytes: Vec<u8>,
    httpserver: Httpserver,
    msgtosend: Vec<u8>,
}

impl<'a> TcpServer<'a> {
    pub fn init_server(seed: u32) -> Self {
        // Create interface
        let mut device = StmPhy::new();
        let mut config = Config::new(EthernetAddress([0x00, 0x80, 0xE1, 0x00, 0x00, 0x01]).into());
        config.random_seed = seed as u64;
        let mut iface = Interface::new(config, &mut device, Instant::from_millis(0));
        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs
                .push(IpCidr::new(IpAddress::v4(192, 168, 69, 1), 24))
                .unwrap();
        });
        iface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::new(192, 168, 69, 100))
            .unwrap();

        // Create sockets
        let tcp1_rx_buffer = tcp::SocketBuffer::new(vec![0; 128]);
        let tcp1_tx_buffer = tcp::SocketBuffer::new(vec![0; 128]);
        let tcp1_socket = tcp::Socket::new(tcp1_rx_buffer, tcp1_tx_buffer);

        let mut sockets = SocketSet::new(vec![]);
        let tcp1_handle = sockets.add(tcp1_socket);

        //build http server
        let mut callbacks: CallbackBt = CallbackBt::new();
        callbacks.insert("GET", &HTTPGETHANDLE); //TODO: upstream?
        callbacks.insert("POST", &HTTPPOSTHANDLE);

        TcpServer {
            device,
            iface,
            sockets,
            tcp1_handle,
            httpserver: Httpserver::new(callbacks),
            rxbytes: Vec::<u8>::new(),
            msgtosend: Vec::<u8>::new(),
        }
    }

    fn run_webserver(&mut self) {
        //get the tcp socket
        let sock = self.sockets.get_mut::<tcp::Socket>(self.tcp1_handle);

        //ensure socket is open.
        if !sock.is_open() {
            sock.listen(80).unwrap();
        }

        // if socket was closed, reset the write pointer.
        if sock.state() == State::CloseWait {
            sock.close()
        }

        if sock.can_send() && !self.msgtosend.is_empty() {
            let sent = sock
                .send_slice(&self.msgtosend[0..])
                .expect("failed to send message");
            self.msgtosend = self.msgtosend[sent..].to_vec();
        }

        if sock.can_recv() && self.msgtosend.is_empty() {
            let mut rxslice = [0u8; RINGBUFSIZE];
            let len = sock.recv_slice(&mut rxslice).expect("failed to receive");

            self.rxbytes.extend_from_slice(&rxslice[0..len]);

            match self.httpserver.parse_request(&self.rxbytes) {
                Ok(resp) => {
                    self.msgtosend = resp;
                    //the assumption here as if we didn't parse the header we don't have all of it.
                    self.rxbytes.clear();
                }
                Err(x) => {
                    if let HttpError::Unsupported = x {
                        warn!("failed to parse request!")
                    }
                }
            };
        }
    }

    fn run_dhcpserver(&mut self) {
        //TODO!
    }

    pub fn eth_task(&mut self, currtime: u32) {
        let _send_at = Instant::from_millis(currtime);
        let _ident: u16 = 0x22b;
        let timestamp = Instant::from_millis(currtime);
        self.iface
            .poll(timestamp, &mut self.device, &mut self.sockets);

        self.run_webserver();
        self.run_dhcpserver();
    }
    pub fn get_bufs(&mut self) -> EthRingBuffers {
        (&mut self.device.rxq, &mut self.device.txq)
    }
}
