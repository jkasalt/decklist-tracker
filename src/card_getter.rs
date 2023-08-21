use anyhow::Result;
use reqwest::blocking::{get, Client};
use serde_json::Value;
use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command};

pub struct CardGetter {
    daemon: Child,
    _listener: TcpListener,
    port: SocketAddr,
}

impl CardGetter {
    pub fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?;
        let daemon = Command::new(
            r"mtga-tracker-daemon\src\mtga-tracker-daemon\bin\Debug\net6.0\mtga-tracker-daemon.exe",
        )
        .args(["-p", &port.to_string()])
        .spawn()?;

        Ok(CardGetter {
            daemon,
            _listener: listener,
            port,
        })
    }

    fn adress(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    pub fn cards(&self) -> Result<Value> {
        let url = dbg!(format!("{}/cards", self.adress()));
        let response = &dbg!(get(url)?.text()?);
        let value = dbg!(serde_json::from_str(response)?);
        Ok(value)
    }

    #[cfg(test)]
    pub fn status(&self) -> Result<Value> {
        let url = format!("{}/status", self.adress());
        let response = &dbg!(get(url)?.text()?);
        let value = dbg!(serde_json::from_str(response)?);
        Ok(value)
    }
}

impl Drop for CardGetter {
    fn drop(&mut self) {
        let url = format!("{}/shutdown", self.adress());
        let _ = Client::new()
            .post(url)
            .send()
            .map_err(|err| eprintln!("ERROR: Failed to call shutdown on daemon, {err}"));
        let _ = self
            .daemon
            .kill()
            .map_err(|err| eprintln!("ERROR: Failed to kill daemon, {err}"));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn get_status() -> Result<()> {
        let card_getter = CardGetter::new(8989)?;
        let status = card_getter.status()?;
        assert_eq!(
            serde_json::json!({
                "daemonVersion": "1.0.6.1",
                "isRunning": "false",
                "processId": -1,
                "updating": "false",
            }),
            status
        );
        Ok(())
    }
}
