use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use log::{info, warn};
use nmea_parser::{NmeaParser, ParsedMessage};
use std::io::{BufRead, BufReader};
use std::thread::{self, spawn, JoinHandle};
use std::time::Duration;

pub struct NmeaConfig {
    pub serial_port_path: String,
    pub baud_rate: u32,
    pub data_tx: Sender<ParsedMessage>,
}

impl NmeaConfig {
    pub fn into_task(self) -> JoinHandle<()> {
        spawn(move || loop {
            if let Err(e) = self.evolve() {
                warn!("Failed to read from cortex: {e:?}");
                thread::sleep(Duration::from_secs(1));
            }
        })
    }

    fn evolve(&self) -> Result<()> {
        let mut parser = NmeaParser::new();

        info!(
            "Opening Nmea on `{}` ({}b/s)",
            &self.serial_port_path, self.baud_rate
        );
        let serial = serialport::new(&self.serial_port_path, self.baud_rate)
            .open_native()
            .with_context(|| format!("Failed to open {}", &self.serial_port_path))?;

        let mut reader = BufReader::new(serial);
        let mut line = String::new();
        loop {
            line.clear();
            reader
                .read_line(&mut line)
                .context("Failed to Nmea sentence line from cortex")?;
            dbg!(&line);
            match parser.parse_sentence(&line) {
                Ok(v) => {
                    dbg!(&v);
                    if self.data_tx.try_send(v).is_err() {
                        warn!("Failed to send Nmea message to ui task");
                    }
                }
                Err(e) => {
                    warn!("Failed to parse cortex line `{line}`: {e:?}");
                    continue;
                }
            };
        }
    }
}
