use core::mem::MaybeUninit;

use embassy_usb::driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointOut};
use embassy_usb::types::StringIndex;
use embassy_usb::{msos, Builder, Handler};
use rp2040_fred_protocol::bridge_proto::{USB_VENDOR_CLASS, USB_VENDOR_SUBCLASS};

pub struct VendorBulkState {
    control: MaybeUninit<VendorBulkControl>,
}

struct VendorBulkControl {
    iface_string: StringIndex,
    iface_name: &'static str,
}

impl VendorBulkState {
    pub const fn new() -> Self {
        Self {
            control: MaybeUninit::uninit(),
        }
    }
}

impl Handler for VendorBulkControl {
    fn get_string(&mut self, index: StringIndex, _lang_id: u16) -> Option<&str> {
        (index == self.iface_string).then_some(self.iface_name)
    }
}

pub struct VendorBulkInterface<'d, D: Driver<'d>> {
    read_ep: D::EndpointOut,
    write_ep: D::EndpointIn,
    max_packet_size: u16,
}

impl<'d, D: Driver<'d>> VendorBulkInterface<'d, D> {
    pub fn new(
        builder: &mut Builder<'d, D>,
        state: &'d mut VendorBulkState,
        protocol: u8,
        iface_name: &'static str,
        device_interface_guid: &'static str,
        max_packet_size: u16,
    ) -> Self {
        let iface_string = builder.string();
        let mut function = builder.function(USB_VENDOR_CLASS, USB_VENDOR_SUBCLASS, protocol);
        function.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));
        function.msos_feature(msos::RegistryPropertyFeatureDescriptor::new(
            "DeviceInterfaceGUIDs",
            msos::PropertyData::RegMultiSz(&[device_interface_guid]),
        ));

        let mut interface = function.interface();
        let mut alt = interface.alt_setting(
            USB_VENDOR_CLASS,
            USB_VENDOR_SUBCLASS,
            protocol,
            Some(iface_string),
        );
        let read_ep = alt.endpoint_bulk_out(None, max_packet_size);
        let write_ep = alt.endpoint_bulk_in(None, max_packet_size);
        drop(function);

        builder.handler(state.control.write(VendorBulkControl {
            iface_string,
            iface_name,
        }));

        Self {
            read_ep,
            write_ep,
            max_packet_size,
        }
    }

    pub async fn wait_connection(&mut self) {
        self.read_ep.wait_enabled().await;
    }

    pub async fn read_packet(&mut self, data: &mut [u8]) -> Result<usize, EndpointError> {
        let mut n = 0;

        loop {
            let i = self.read_ep.read(&mut data[n..]).await?;
            n += i;
            if i < self.max_packet_size as usize {
                return Ok(n);
            }
        }
    }

    pub async fn write_packet(&mut self, data: &[u8]) -> Result<(), EndpointError> {
        for chunk in data.chunks(self.max_packet_size as usize) {
            self.write_ep.write(chunk).await?;
        }
        if data.len() % self.max_packet_size as usize == 0 {
            self.write_ep.write(&[]).await?;
        }
        Ok(())
    }
}
