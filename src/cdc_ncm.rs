use core::mem::size_of;

extern crate alloc;
use alloc::vec::Vec;
use defmt::debug;
//cdc_ncm
//an implmentation of the mcm mode for cdc
use num_enum::TryFromPrimitive;
// use serde::Serialize;
use usb_device::class_prelude::*;
use usb_device::Result;

/// This should be used as `device_class` when building the `UsbDevice`.

//FIXME: a lot of these can be tkaen from original usb_acm rather than redefing..
pub const USB_CLASS_CDC: u8 = 0x02;

const USB_CLASS_CDC_DATA: u8 = 0x0a;
pub const CDC_SUBCLASS_NCM: u8 = 0x0D;
const CDC_PROTOCOL_NONE: u8 = 0x00;

const CS_INTERFACE: u8 = 0x24;
const CDC_TYPE_HEADER: u8 = 0x00;

const CDC_TYPE_UNION: u8 = 0x06;

const ETH_NET_FUNC_DESC: u8 = 0x0f;

pub const NCM_MAX_SEGMENT_SIZE: usize = 1514;

const USBD_ISTR_INTERFACES: u8 = 0x00;

pub const NCM_MAX_IN_SIZE: usize = 2048;
pub const NCM_MAX_OUT_SIZE: usize = 2048;

pub const NTH16_SIGNATURE: &[u8] = "NCMH".as_bytes();
pub const NDP16_SIGNATURE: &[u8] = "NCM0".as_bytes();

#[repr(u8)]
#[derive(Default, PartialEq)]
pub enum NTBState {
    #[default]
    Empty = 0,
    Processing = 1,
    Transferring = 2,
    Ready = 3,
}

#[derive(Debug, defmt::Format, TryFromPrimitive)]
#[repr(u8)]
enum CDCRequests {
    GetNTBParameters = 128,
    GetNTBInputSize = 133,
    SetNTBInputSize = 134,
}

pub struct CdcNcmClass<'a, B: UsbBus> {
    comm_if: InterfaceNumber,
    ned_ep: EndpointIn<'a, B>,
    data_if: InterfaceNumber,
    read_ep: EndpointOut<'a, B>,
    write_ep: EndpointIn<'a, B>,
    namestr: StringIndex,
    macaddrstr: StringIndex,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct NCMTransferHeader {
    pub signature: u32,
    pub headerlen: u16,
    pub sequence: u16,
    pub blocklen: u16,
    pub ndpidex: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct NCMDatagram16 {
    pub index: u16,
    pub length: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct NCMDatagramPointerTable {
    pub signature: u32,
    pub length: u16,
    pub nextndpindex: u16,
    pub datagrams: Vec<NCMDatagram16>,
}

#[repr(C, packed)]
#[derive(Default)]
struct NCMParameters {
    length: u16,                   /* Size in bytes of this NTBT structure */
    ntb_formats_supported: u16,    /* 1 if only 16bit, 3 if 32bit is supported as well */
    ntb_in_maxsize: u32,           /* IN NTB Maximum Size in bytes */
    ndp_in_divisor: u16,           /* Divisor used for IN NTB Datagram payload alignment */
    ndp_in_payload_remainder: u16, /* Remainder used to align input datagram payload within the NTB */
    ndp_in_alignment: u16,         /* Datagram alignment */
    reserved: u16,                 /* Keep 0 */
    ntb_out_maxsize: u32,
    ndp_out_divisor: u16,
    ndp_out_payload_remainder: u16,
    ndp_out_alignment: u16,
    ntb_out_max_datagrams: u16, /* Maximum number of datagrams in a single OUT NTB */
}

impl<B: UsbBus> CdcNcmClass<'_, B> {
    /// Creates a new CdcAcmClass with the provided UsbBus and max_packet_size in bytes. For
    pub fn new(alloc: &UsbBusAllocator<B>) -> CdcNcmClass<'_, B> {
        CdcNcmClass {
            comm_if: alloc.interface(),
            ned_ep: alloc.interrupt(32, 20),
            data_if: alloc.interface(),
            read_ep: alloc.bulk(64),
            write_ep: alloc.bulk(64),
            namestr: alloc.string(),
            macaddrstr: alloc.string(),
        }
    }

    /// Gets the maximum packet size in bytes.
    pub fn max_packet_size(&self) -> u16 {
        // The size is the same for both endpoints.
        self.read_ep.max_packet_size()
    }

    /// Writes a single packet into the IN endpoint.
    pub fn write_packet(&mut self, data: &[u8]) -> Result<usize> {
        self.write_ep.write(data)
    }

    /// Reads a single packet from the OUT endpoint.
    pub fn read_packet(&mut self, data: &mut [u8]) -> Result<usize> {
        self.read_ep.read(data)
    }

    /// Gets the address of the IN endpoint.
    pub(crate) fn write_ep_address(&self) -> EndpointAddress {
        self.write_ep.address()
    }

    pub fn send_notification(&mut self, data: &[u8]) -> Result<usize> {
        self.ned_ep.write(data)
    }
}

impl<B: UsbBus> UsbClass<B> for CdcNcmClass<'_, B> {
    fn get_string(&self, index: StringIndex, lang_id: u16) -> Option<&str> {
        match index.into() {
            4 => Some("IP Gateway"),
            5 => Some("0080E1000000"),
            _ => None,
        }
    }

    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        /* Interface Association Descriptor */
        writer.iad(
            self.comm_if,
            2,
            USB_CLASS_CDC,
            CDC_SUBCLASS_NCM,
            CDC_PROTOCOL_NONE,
        )?;

        /* Comm Interface Descriptor */
        writer.interface(
            self.comm_if,
            USB_CLASS_CDC,
            CDC_SUBCLASS_NCM,
            CDC_PROTOCOL_NONE,
        )?;

        /* Header Functional Descriptor */
        writer.write(
            CS_INTERFACE,
            &[
                CDC_TYPE_HEADER, // bDescriptorSubtype
                0x10,
                0x01, // bcdCDC (1.10)
            ],
        )?;

        /* Union Functional Descriptor */
        writer.write(
            CS_INTERFACE,
            &[CDC_TYPE_UNION, self.comm_if.into(), self.data_if.into()],
        )?;

        /* Ethernet Networking Functional Descriptor */
        writer.write(
            CS_INTERFACE,
            &[
                ETH_NET_FUNC_DESC,
                self.macaddrstr.into(),                     //imacaddress
                0x0,                                        //eth stats
                0x0,                                        //eth stats
                0x0,                                        //eth stats
                0x0,                                        //eth stats
                (NCM_MAX_SEGMENT_SIZE & 0x0000_00ff) as u8, //max segment size
                ((NCM_MAX_SEGMENT_SIZE & 0x0000_ff00) >> 8) as u8, //max segment size
                0x0,                                        //mc filters?
                0x0,
                0x0, //power filters..?
            ],
        )?;

        /* NCM Functional Descriptor */
        writer.write(
            CS_INTERFACE,
            &[
                0x1A, //ncm func desc
                0x00, 0x01, //ncm version
                0x00, //network capabilites
            ],
        )?;

        /* Notification Endpoint Descriptor */
        writer.endpoint(&self.ned_ep)?;

        writer.interface_alt(self.data_if, 0, USB_CLASS_CDC_DATA, 0, 0x01, None)?;
        writer.interface_alt(
            self.data_if,
            1,
            USB_CLASS_CDC_DATA,
            0,
            0x01,
            Some(self.namestr),
        )?;

        writer.endpoint(&self.read_ep)?;
        writer.endpoint(&self.write_ep)?;

        Ok(())
    }

    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = xfer.request();
        let data = xfer.data();

        if req.request_type == control::RequestType::Class {
            debug!("got request {}", req.request);
            if let Ok(_request) = CDCRequests::try_from_primitive(req.request) {
                // gracefully accept the transfer and skip for now.
                debug!("handled request {:?}", _request);
                xfer.accept().ok();
            } else {
                debug!("uhandled out request {}", req.request);
                xfer.reject().ok();
            }
        }
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = xfer.request();

        if req.request_type == control::RequestType::Class {
            debug!("got request {}", req.request);
            if let Ok(request) = CDCRequests::try_from_primitive(req.request) {
                match request {
                    CDCRequests::GetNTBParameters => {
                        xfer.accept(|data| {
                            const LEN: usize = size_of::<NCMParameters>();
                            const PARAMS: NCMParameters = NCMParameters {
                                length: LEN as u16,
                                ntb_formats_supported: 1,
                                ntb_in_maxsize: NCM_MAX_IN_SIZE as u32,
                                ndp_in_divisor: 4,
                                ndp_in_alignment: 4,
                                ndp_in_payload_remainder: 0,
                                ntb_out_maxsize: NCM_MAX_OUT_SIZE as u32,
                                ndp_out_divisor: 4,
                                ndp_out_alignment: 4,
                                ndp_out_payload_remainder: 4,
                                ntb_out_max_datagrams: 20,
                                reserved: 0,
                            };

                            data[0..2].copy_from_slice(&PARAMS.length.to_le_bytes());
                            data[2..4].copy_from_slice(&PARAMS.ntb_formats_supported.to_le_bytes());
                            data[4..8].copy_from_slice(&PARAMS.ntb_in_maxsize.to_le_bytes());
                            data[8..10].copy_from_slice(&PARAMS.ndp_in_divisor.to_le_bytes());
                            data[10..12]
                                .copy_from_slice(&PARAMS.ndp_in_payload_remainder.to_le_bytes());
                            data[12..14].copy_from_slice(&PARAMS.ndp_in_alignment.to_le_bytes());
                            data[14..16].copy_from_slice(&PARAMS.reserved.to_le_bytes());
                            data[16..20].copy_from_slice(&PARAMS.ntb_out_maxsize.to_le_bytes());
                            data[20..22].copy_from_slice(&PARAMS.ndp_out_divisor.to_le_bytes());
                            data[22..24]
                                .copy_from_slice(&PARAMS.ndp_out_payload_remainder.to_le_bytes());
                            data[24..26].copy_from_slice(&PARAMS.ndp_out_alignment.to_le_bytes());
                            data[26..28]
                                .copy_from_slice(&PARAMS.ntb_out_max_datagrams.to_le_bytes());

                            Ok(LEN)
                        })
                        .ok();
                    }
                    CDCRequests::GetNTBInputSize => {
                        xfer.accept(|data| {
                            data[0..3]
                                .copy_from_slice(&self.write_ep.max_packet_size().to_le_bytes());
                            Ok(4)
                        })
                        .ok();
                    }
                    CDCRequests::SetNTBInputSize => {
                        xfer.accept(|_data| Ok(0)).ok();
                    }
                }
            } else {
                debug!("uhandled in request {}", req.request);
                xfer.reject().ok();
            }
        }
    }
    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        if interface == self.data_if {
            Some(1)
        } else {
            None
        }
    }
    fn set_alt_setting(&mut self, interface: InterfaceNumber, alternative: u8) -> bool {
        (interface,alternative) == (self.data_if,1)

    }
}
