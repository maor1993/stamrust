extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use defmt::warn;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp;
use smoltcp::socket::tcp::State;
use smoltcp::time::Instant;
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address};

use crate::ncm_netif::{EthRingBuffers, StmPhy};
use defmt::info;

use crate::http::{
    gen_http_header, CallbackBt, HttpCallback, HttpContentType, HttpEncodingType, HttpRequest,
    Httpserver, HTTP_404_RESPONSE,
};

const TESTWEBSITE: &[u8] = include_bytes!("../static/mockup_mini.html.gz");

struct ServeWebSite {
    data: &'static [u8],
}

impl HttpCallback for ServeWebSite {
    fn handle_request(&self, request: &HttpRequest) -> Vec<u8> {
        info!("{}",request);
        match request.path.as_str() {
            "/" | "/index.html" => {
                let mut buf: Vec<u8> = gen_http_header(
                    self.data,
                    HttpContentType::Text,
                    Some(HttpEncodingType::Gzip),
                );
                buf.extend_from_slice(TESTWEBSITE);
                buf
            }
            _ => HTTP_404_RESPONSE.into(),
        }
    }
}

const WEBSITESERVER: ServeWebSite = ServeWebSite { data: TESTWEBSITE };

struct HandleRgb;

pub struct TcpServer<'a> {
    device: StmPhy,
    iface: Interface,
    sockets: SocketSet<'a>,
    tcp1_handle: SocketHandle,
    data_rem: usize,
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
        callbacks.insert("GET", &WEBSITESERVER); //TODO: upstream?

        TcpServer {
            device,
            iface,
            sockets,
            tcp1_handle,
            data_rem: 0,
            httpserver: Httpserver::new(callbacks),
            msgtosend: vec![0u8; 256],
        }
    }

    fn handle_http_requests(&mut self) {
        //get the tcp socket
        let sock = self.sockets.get_mut::<tcp::Socket>(self.tcp1_handle);

        //ensure socket is open.
        if !sock.is_open() {
            sock.listen(80).unwrap();
        }

        // if socket was closed, reset the write pointer.
        if sock.state() == State::CloseWait {
            self.data_rem = 0;
            sock.close()
        }

        if sock.can_send() && !self.msgtosend.is_empty() {
            let sent = sock
                .send_slice(&self.msgtosend[0..])
                .expect("failed to send message");
            self.msgtosend = self.msgtosend[sent..].to_vec();
        }

        if sock.can_recv() && self.msgtosend.is_empty() {
            let mut slice = [0; 256];
            let _bytecnt = sock.recv_slice(&mut slice).expect("failed to receive");

            match self.httpserver.parse_request(&slice) {
                Ok(resp) => {
                    self.msgtosend=resp;
                }
                Err(_x) => warn!("failed to parse request!"),
            };
        }
    }

    pub fn eth_task(&mut self, currtime: u32) {
        let _send_at = Instant::from_millis(currtime);
        let _ident: u16 = 0x22b;
        let timestamp = Instant::from_millis(currtime);
        self.iface
            .poll(timestamp, &mut self.device, &mut self.sockets);

        self.handle_http_requests();
    }
    pub fn get_bufs(&mut self) -> EthRingBuffers {
        (&mut self.device.rxq, &mut self.device.txq)
    }
}
