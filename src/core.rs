use std::{
    error::Error,
    net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket},
    thread,
    time::Duration,
};

use stunclient::StunClient;

pub struct Updater<T>
where
    T: ApiTrait,
{
    api: T,
}

impl<T: ApiTrait> Updater<T> {
    pub fn new(poll_rate: usize, api_key: String) -> Self {
        Self {
            api: T::new(poll_rate, api_key),
        }
    }

    pub fn run(&mut self) {
        loop {
            let current_ip = match self.api.check_ip() {
                Ok(ip) => ip,
                Err(e) => {
                    eprintln!("Failed to check IP: {}", e);
                    continue;
                }
            };
            let records = self.api.get_records().clone();
            for record in records.iter() {
                let mut record_clone = record.clone();
                // implement a way of checking if something on the host has changed and update the record if it has
                // for now, just update the record if the IP has changed
                if record.get_content() != current_ip.ip() {
                    // create a new record with the new IP
                    record_clone = record_clone.update_content(current_ip.ip());
                }
                if !record_clone.eq(record) {
                    self.api.update_record(&record_clone);
                }
            }
            thread::sleep(Duration::from_secs(self.api.get_poll_rate() as u64));
        }
    }
}

pub trait ApiTrait: Sized {
    type RecordType: Record<Self> + Clone;

    fn new(poll_rate: usize, api_key: String) -> Self;
    fn check_ip(&self) -> Result<SocketAddr, Box<dyn Error>> {
        let local_addr: SocketAddr = "0.0.0.0:0".parse()?;
        let udp = UdpSocket::bind(local_addr)?;
        let stun_server = "stun.cloudflare.com:3478"
            .to_socket_addrs()?
            .find(|x| x.is_ipv4())
            .ok_or("No IPv4 address found for STUN server")?;

        let c = StunClient::new(stun_server);
        let addr = c.query_external_address(&udp)?;
        Ok(addr)
    }
    fn update_record(&mut self, record: &Self::RecordType) -> Self::RecordType;
    fn get_records(&mut self) -> &Vec<Self::RecordType>;
    fn get_poll_rate(&self) -> usize;
}

pub trait Record<T: ApiTrait>: 'static {
    fn get_id(&self) -> Option<String>;
    fn get_name(&self) -> String;
    fn get_content(&self) -> IpAddr;
    fn update_content(&self, new_content: IpAddr) -> Self;

    fn eq(&self, other: &Self) -> bool {
        self.get_content() == other.get_content()
            && self.get_id() == other.get_id()
            && self.get_name() == other.get_name()
    }
}

pub trait Config {
    fn get_poll_rate() -> usize;
}
