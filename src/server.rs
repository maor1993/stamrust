extern crate alloc;

use core::cell::RefMut;

use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities};
use smoltcp::socket::tcp::State;
use smoltcp::socket::{icmp, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr};
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address};

use crate::ncm_netif::{StmPhy, SyncBuf};
use defmt::{debug, info};

const TESTWEBSITE: &[u8] = include_bytes!("../static/index.html");

pub struct TcpServer<'a> {
    device: StmPhy,
    iface: Interface,
    sockets: SocketSet<'a>,
    tcp1_handle: SocketHandle,
    icmp_handle: SocketHandle,
    curr_data_idx: usize,
}

impl<'a> TcpServer<'a> {
    pub fn init_server(seed: u32) -> Self {
        // Create interface
        let mut device = StmPhy::new();
        let mut config = Config::new(EthernetAddress([0x00, 0x80, 0xE1, 0x00, 0x00, 0x00]).into());
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

        let icmp_rx_buffer =
            icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY], vec![0; 256]);
        let icmp_tx_buffer =
            icmp::PacketBuffer::new(vec![icmp::PacketMetadata::EMPTY], vec![0; 256]);
        let icmp_socket = icmp::Socket::new(icmp_rx_buffer, icmp_tx_buffer);

        // let dhcp_config = dhcpv4::Config{address:Ipv4Cidr::new(, 24)};

        // dhcp_socket.

        let mut sockets = SocketSet::new(vec![]);
        let tcp1_handle = sockets.add(tcp1_socket);
        let icmp_handle = sockets.add(icmp_socket);
        TcpServer {
            device,
            iface,
            sockets,
            tcp1_handle,
            icmp_handle,
            curr_data_idx: 0,
        }
    }
    fn handle_web_requests(sock: &mut tcp::Socket<'_>,mut last_tx_idx:usize ) -> usize{
        if !sock.is_open() {
            sock.listen(6969).unwrap();
            
        }

        if  sock.state() == State::CloseWait {
            last_tx_idx = 0;
            sock.close()
        }


        if sock.can_send() && last_tx_idx < TESTWEBSITE.len() {
            last_tx_idx += sock
                .send_slice(&TESTWEBSITE[last_tx_idx..])
                .expect("failed to send message");
        }

        if sock.can_recv(){
            let mut slice = [0;256];
            let bytecnt = sock.recv_slice(&mut slice).expect("failed to receive");
            info!("recv bytes: {}",slice[0..bytecnt]);
        }
        last_tx_idx
        
    }


    pub fn eth_task(&mut self, currtime: u32) {
        let mut send_at = Instant::from_millis(currtime);
        let ident: u16 = 0x22b;
        let timestamp = Instant::from_millis(currtime);
        self.iface
            .poll(timestamp, &mut self.device, &mut self.sockets);

        let tcp_socket = self.sockets.get_mut::<tcp::Socket>(self.tcp1_handle);
        
        self.curr_data_idx = Self::handle_web_requests(tcp_socket,self.curr_data_idx);
        

        
    
    }
    pub fn get_bufs(&mut self) -> (RefMut<SyncBuf>, RefMut<SyncBuf>) {
        (
            self.device.rxbuf.borrow_mut(),
            self.device.txbuf.borrow_mut(),
        )
    }
}
