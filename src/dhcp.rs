use core::cmp;
use core::mem;
use core::mem::size_of;

use alloc::vec;
use defmt::info;
use num_enum::IntoPrimitive;
use num_enum::TryFromPrimitive;
use smoltcp::wire::Ipv4Address;

extern crate alloc;
use alloc::vec::Vec;

pub const DHCP_SERVER_PORT: u16 = 67;
pub const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_CHADDR_LEN: usize = 16;
const DHCP_SNAME_LEN: usize = 64;
const DHCP_FILE_LEN: usize = 128;
const DHCP_OPTIONS_LEN: usize = 68;

const DHCP_MAGIC_COOKIE: u32 = 0x63825363;

#[derive(Debug, Clone, Copy, defmt::Format, IntoPrimitive)]
#[repr(u8)]
enum DhcpOpcodes {
    BootRequest = 1,
    BootReply = 2,
}

#[derive(Debug, Clone, Copy, defmt::Format, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
enum DhcpMsgTypes {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

#[derive(Debug, Clone, Copy, defmt::Format, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
enum DhcpOptionTypes {
    Pad = 0,
    Subnetmask = 1,
    Router = 3,
    Dnsserver = 6,
    Hostname = 12,
    Ipttl = 23,
    Mtu = 26,
    Broadcast = 28,
    Tcpttl = 37,
    Ntp = 42,
    Requestedip = 50,
    Leasetime = 51,
    Overload = 52,
    MsgType = 53,
    ServerId = 54,
    End = 255,
}
#[derive(defmt::Format, Debug)]
#[repr(C, packed(4))]
struct DhcpMsg {
    op: u8,
    htype: u8,
    hlen: u8,
    hops: u8,
    xid: u32,
    secs: u16,
    flags: u16,
    ciaddr: Ipv4Address,
    yiaddr: Ipv4Address,
    siaddr: Ipv4Address,
    giaddr: Ipv4Address,
    chaddr: [u8; DHCP_CHADDR_LEN],
    sname: [u8; DHCP_SNAME_LEN],
    file: [u8; DHCP_FILE_LEN],
    cookie: u32,
    options: [u8; DHCP_OPTIONS_LEN],
}

trait ToBytes {
    fn conv_to_bytes(&self) -> Vec<u8>;
}

impl ToBytes for Vec<OptionU32Msg> {
    fn conv_to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::<u8>::new();
        self.iter().for_each(|x| {
            let buf: [u8; 6] = x.into();
            out.extend_from_slice(&buf[0..2 + (x.len as usize)])
        });
        out
    }
}

#[derive(defmt::Format, Debug)]
struct OptionU32Msg {
    type_: DhcpOptionTypes,
    len: u8,
    data: [u8; 4],
}

impl From<&OptionU32Msg> for [u8; 6] {
    fn from(value: &OptionU32Msg) -> Self {
        let mut arr = [0u8; 6];
        arr[0] = value.type_.into();
        arr[1] = value.len;
        arr[2..].copy_from_slice(&value.data);
        arr
    }
}
impl From<&[u8]> for OptionU32Msg {
    fn from(value: &[u8]) -> Self {
        let type_ = DhcpOptionTypes::try_from_primitive(value[0]).unwrap();
        let len = value[1];
        let mut data = [0u8; 4];
        data[0..(len as usize)].copy_from_slice(&value[2..2 + (len as usize)]);
        OptionU32Msg { type_, len, data }
    }
}

impl Default for DhcpMsg {
    fn default() -> Self {
        DhcpMsg {
            op: 0,
            htype: 0,
            hlen: 0,
            hops: 0,
            xid: 0,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Address::default(),
            yiaddr: Ipv4Address::default(),
            siaddr: Ipv4Address::default(),
            giaddr: Ipv4Address::default(),
            chaddr: [0u8; DHCP_CHADDR_LEN],
            sname: [0u8; DHCP_SNAME_LEN],
            file: [0u8; DHCP_FILE_LEN],
            cookie: DHCP_MAGIC_COOKIE,
            options: [0u8; DHCP_OPTIONS_LEN],
        }
    }
}

impl From<&[u8]> for DhcpMsg {
    fn from(value: &[u8]) -> Self {
        const FILESTART: usize = 44 + DHCP_SNAME_LEN;
        const COOKIESTART: usize = 44 + DHCP_SNAME_LEN + DHCP_FILE_LEN;
        const OPTIONSTART: usize = 44 + DHCP_SNAME_LEN + DHCP_FILE_LEN + 4;
        const OPTIONSEND: usize = OPTIONSTART + DHCP_OPTIONS_LEN;
        let opts: [u8; DHCP_OPTIONS_LEN] = match value.len().cmp(&OPTIONSEND) {
            cmp::Ordering::Less => {
                let mut optsbuf = [0u8; DHCP_OPTIONS_LEN];
                optsbuf[0..(value.len() - OPTIONSTART)]
                    .copy_from_slice(&value[OPTIONSTART..value.len()]);
                optsbuf
            }
            _ => value[OPTIONSTART..OPTIONSEND].try_into().unwrap(),
        };

        DhcpMsg {
            op: value[0],
            htype: value[1],
            hlen: value[2],
            hops: value[3],
            xid: u32::from_le_bytes(value[4..8].try_into().unwrap()),
            secs: u16::from_le_bytes(value[8..10].try_into().unwrap()),
            flags: u16::from_le_bytes(value[10..12].try_into().unwrap()),
            ciaddr: Ipv4Address::from_bytes(value[12..16].try_into().unwrap()),
            yiaddr: Ipv4Address::from_bytes(value[16..20].try_into().unwrap()),
            siaddr: Ipv4Address::from_bytes(value[20..24].try_into().unwrap()),
            giaddr: Ipv4Address::from_bytes(value[24..28].try_into().unwrap()),
            chaddr: value[28..44].try_into().unwrap(),
            sname: value[44..FILESTART].try_into().unwrap(),
            file: value[FILESTART..COOKIESTART].try_into().unwrap(),
            cookie: u32::from_le_bytes(value[COOKIESTART..OPTIONSTART].try_into().unwrap()),
            options: opts,
        }
    }
}
impl Into<Vec<u8>> for DhcpMsg {
    fn into(self) -> Vec<u8> {
        // let mut buf = vec![self.op, self.htype, self.hlen, self.hops];
        let buf: [u8; size_of::<DhcpMsg>()] = unsafe { mem::transmute(self) };
        buf.to_vec()
    }
}

#[derive(Default)]
pub struct DhcpServer {
    pub addrstart: u8,
    pub maxaddr: u8,
    pub addrcnt: u8,
    pub serverip: Ipv4Address,
    pub subnet: Ipv4Address,
    pub allocated: Vec<[u8; 6]>, // supports only EUI-48 addresses.
}

impl DhcpServer {
    pub fn recv(&mut self, buf: &[u8]) -> Option<Vec<u8>> {
        let incoming: DhcpMsg = buf.into();
        // info!("msg: {:?}", incoming);

        //convert the first section into an option
        let req_opt: OptionU32Msg = incoming.options[0..6].into();
        info!("req: {:?}", req_opt);
        match DhcpMsgTypes::try_from_primitive(req_opt.data[0]).unwrap() {
            DhcpMsgTypes::Discover => {
                Some(self.create_dhcp_reply(incoming, DhcpMsgTypes::Offer).into())
            }
            DhcpMsgTypes::Request => {
                Some(self.create_dhcp_reply(incoming, DhcpMsgTypes::Ack).into())
            }
            _ => None,
        }
    }

    fn create_dhcp_reply(&mut self, incoming: DhcpMsg, msg_type: DhcpMsgTypes) -> DhcpMsg {
        let mut options = Vec::<OptionU32Msg>::new();
        let mut ipbuf: [u8; 4] = [0u8; 4];

        //create the option info
        // step 1: header.
        options.push(OptionU32Msg {
            type_: DhcpOptionTypes::MsgType,
            len: 1,
            data: [msg_type.into(), 0, 0, 0],
        });

        //subnet mask
        ipbuf.copy_from_slice(self.subnet.as_bytes());
        options.push(OptionU32Msg {
            type_: DhcpOptionTypes::Subnetmask,
            len: 4,
            data: ipbuf,
        });

        //gateway
        ipbuf.copy_from_slice(self.serverip.as_bytes());
        options.push(OptionU32Msg {
            type_: DhcpOptionTypes::Router,
            len: 4,
            data: ipbuf,
        });

        //server id
        // ipbuf.copy_from_slice(self.serverip.as_bytes());
        options.push(OptionU32Msg {
            type_: DhcpOptionTypes::ServerId,
            len: 4,
            data: ipbuf,
        });

        //lease time
        options.push(OptionU32Msg {
            type_: DhcpOptionTypes::Leasetime,
            len: 4,
            data: 86400u32.to_le_bytes(),
        });

        //dns server
        options.push(OptionU32Msg {
            type_: DhcpOptionTypes::Dnsserver,
            len: 4,
            data: ipbuf,
        });

        //convert to vec<u8>
        let mut optionbytes: Vec<u8> = options.conv_to_bytes();
        //close the buffer
        optionbytes.push(DhcpOptionTypes::End.into());

        let mut options: [u8; DHCP_OPTIONS_LEN] = [0u8; DHCP_OPTIONS_LEN];

        options[0..optionbytes.len()].copy_from_slice(optionbytes.as_slice());
        DhcpMsg {
            op: DhcpOpcodes::BootReply.into(),
            secs: 0,
            flags: 0,
            options,
            yiaddr: self.create_lease(&incoming.chaddr[0..6]),
            ..incoming
        }
        //TODO: when parsing message do not give new leases to users with ip
    }

    fn create_lease(&mut self, requester: &[u8]) -> Ipv4Address {
        let mut buf = [0u8;6];
        buf.copy_from_slice(requester);
        let mut ip = self.serverip;
        // if this requester already was leased an address, return the same address.
        let idx: Vec<usize> = self.allocated.iter().enumerate().filter(|(_, x)| x.as_slice() == requester).map(|(idx,_)| idx).collect::<Vec<_>>();
        



        if idx.len() > 1{
            panic!("somehow we allocated two ips to 1 mac address");
        }

        if idx.is_empty(){
            ip.0[3] = self.addrstart + self.addrcnt;
            self.addrcnt += 1;
            self.allocated.push(buf);
            
        }
        else{
            ip.0[3] =self.addrstart + idx[0] as u8;
        }
        ip
    }
}
