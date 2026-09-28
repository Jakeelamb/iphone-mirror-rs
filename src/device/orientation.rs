//! Read-only interface orientation over the already authenticated native tunnel.
use anyhow::{Context, Result};
use idevice::{
    Idevice, ReadWrite, springboardservices::SpringBoardServicesClient, tcp::handle::AdapterHandle,
};

pub struct OrientationSource {
    adapter: AdapterHandle,
    port: u16,
    client: Option<SpringBoardServicesClient>,
}

impl OrientationSource {
    pub(super) fn new(adapter: AdapterHandle, port: u16) -> Self {
        Self {
            adapter,
            port,
            client: None,
        }
    }

    /// The caller bounds each operation and discards the connection after a
    /// timeout: canceling plist I/O can otherwise leave a partial response.
    pub async fn poll(&mut self) -> Result<u32> {
        if self.client.is_none() {
            let stream: Box<dyn ReadWrite> = Box::new(self.adapter.connect(self.port).await?);
            let mut device = Idevice::new(stream, "iphone-mirror-rs");
            device
                .rsd_checkin()
                .await
                .map_err(|_| anyhow::anyhow!("orientation service check-in failed"))?;
            self.client = Some(SpringBoardServicesClient::new(device));
        }
        let orientation = self
            .client
            .as_mut()
            .context("orientation service is unavailable")?
            .get_interface_orientation()
            .await
            .map_err(|_| anyhow::anyhow!("orientation service query failed"))?;
        Ok(u32::from(orientation as u8))
    }

    pub fn reset(&mut self) {
        self.client = None;
    }
}
