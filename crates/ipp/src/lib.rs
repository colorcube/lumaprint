//! Generic IPP Printer Status & Marker-Level Monitor (`L4`).
//!
//! Queries `marker-names`, `marker-colors`, and `marker-levels` generically over IPP
//! (`localhost:631/printers/<queue>` or remote `ipp://` device URIs) for any installed printer.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use lumaprint_color::Rgb8;
use lumaprint_i18n::tr;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IppError {
    #[error("could not connect to IPP endpoint '{host}:{port}': {reason}")]
    Connect {
        host: String,
        port: u16,
        reason: String,
    },
    #[error("network I/O error: {0}")]
    Io(String),
    #[error("invalid IPP response: {0}")]
    Protocol(String),
}

/// Single ink/toner marker slot status (`marker-names`, `marker-colors`, `marker-levels`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InkChannelLevel {
    pub slot: u8,
    pub code: String,
    pub name: String,
    /// Fill level in percent (`0..=100`).
    pub level_percent: u8,
    pub swatch_rgb: Rgb8,
}

/// Live status of an installed printer queried via IPP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterStatus {
    pub printer_uri: String,
    pub online: bool,
    pub state_summary: String,
    pub channels: Vec<InkChannelLevel>,
}

impl Default for PrinterStatus {
    fn default() -> Self {
        Self::idle_unqueried("ipp://localhost:631/printers/default")
    }
}

impl PrinterStatus {
    pub fn idle_unqueried(printer_uri: &str) -> Self {
        Self {
            printer_uri: printer_uri.to_string(),
            online: false,
            state_summary: tr!("No IPP ink levels queried yet").to_string(),
            channels: Vec::new(),
        }
    }
}

/// Queries live `marker-names`, `marker-colors`, and `marker-levels` from an IPP endpoint (`ipp://host[:port]/path`).
pub fn query_printer_status_ipp(
    printer_uri_or_host: &str,
    timeout_ms: u64,
) -> Result<PrinterStatus, IppError> {
    let (host, port, path) = parse_ipp_target(printer_uri_or_host);
    let addr_str = format!("{host}:{port}");
    let sock_addr: SocketAddr = addr_str
        .to_socket_addrs()
        .map_err(|e| IppError::Connect {
            host: host.clone(),
            port,
            reason: e.to_string(),
        })?
        .next()
        .ok_or_else(|| IppError::Connect {
            host: host.clone(),
            port,
            reason: "no socket address resolved".into(),
        })?;

    let timeout = Duration::from_millis(timeout_ms.max(200));
    let mut stream =
        TcpStream::connect_timeout(&sock_addr, timeout).map_err(|e| IppError::Connect {
            host: host.clone(),
            port,
            reason: e.to_string(),
        })?;
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let full_uri = format!("ipp://{host}:{port}{path}");
    let ipp_req = build_get_printer_attributes_ipp(&full_uri);
    let http_req = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Content-Type: application/ipp\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        ipp_req.len()
    );

    stream
        .write_all(http_req.as_bytes())
        .and_then(|_| stream.write_all(&ipp_req))
        .map_err(|e| IppError::Io(e.to_string()))?;

    let mut resp = Vec::with_capacity(8192);
    stream
        .take(256 * 1024)
        .read_to_end(&mut resp)
        .map_err(|e| IppError::Io(e.to_string()))?;

    parse_ipp_marker_response(&full_uri, &resp)
}

fn parse_ipp_target(uri_or_host: &str) -> (String, u16, String) {
    let trimmed = uri_or_host.trim();
    let without_scheme = trimmed
        .strip_prefix("ipp://")
        .or_else(|| trimmed.strip_prefix("ipps://"))
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);

    let (host_port, path) = match without_scheme.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (without_scheme, String::from("/ipp/print")),
    };

    let (host, port) = match host_port.split_once(':') {
        Some((h, p_str)) => (h.to_string(), p_str.parse::<u16>().unwrap_or(631)),
        None => (
            if host_port.is_empty() {
                String::from("localhost")
            } else {
                host_port.to_string()
            },
            631,
        ),
    };

    (host, port, path)
}

/// Builds an RFC 8011 `Get-Printer-Attributes` (`0x000B`) IPP request payload.
pub fn build_get_printer_attributes_ipp(printer_uri: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(256);
    b.extend_from_slice(&[0x02, 0x00]); // IPP 2.0
    b.extend_from_slice(&0x000Bu16.to_be_bytes()); // Get-Printer-Attributes
    b.extend_from_slice(&1u32.to_be_bytes()); // request-id = 1
    b.push(0x01); // operation-attributes-tag

    write_ipp_attr(&mut b, 0x47, "attributes-charset", b"utf-8");
    write_ipp_attr(&mut b, 0x48, "attributes-natural-language", b"en");
    write_ipp_attr(&mut b, 0x45, "printer-uri", printer_uri.as_bytes());
    write_ipp_attr(&mut b, 0x44, "requested-attributes", b"marker-names");
    write_ipp_attr(&mut b, 0x44, "", b"marker-colors");
    write_ipp_attr(&mut b, 0x44, "", b"marker-levels");
    write_ipp_attr(&mut b, 0x44, "", b"printer-state");
    b.push(0x03); // end-of-attributes-tag
    b
}

fn write_ipp_attr(buf: &mut Vec<u8>, tag: u8, name: &str, val: &[u8]) {
    buf.push(tag);
    buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
    buf.extend_from_slice(name.as_bytes());
    buf.extend_from_slice(&(val.len() as u16).to_be_bytes());
    buf.extend_from_slice(val);
}

/// Parses generic `marker-levels` (and optional `marker-names`) from an IPP response body.
pub fn parse_ipp_marker_response(
    printer_uri: &str,
    raw_http_or_ipp: &[u8],
) -> Result<PrinterStatus, IppError> {
    let body = if let Some(pos) = raw_http_or_ipp.windows(4).position(|w| w == b"\r\n\r\n") {
        raw_http_or_ipp.get(pos + 4..).unwrap_or(raw_http_or_ipp)
    } else {
        raw_http_or_ipp
    };
    if body.len() < 9 {
        return Err(IppError::Protocol("IPP body shorter than 9 bytes".into()));
    }

    let mut levels = Vec::new();
    if let Some(pos) = body.windows(13).position(|w| w == b"marker-levels") {
        let mut cursor = pos + 13;
        if let Some(v_bytes) = body.get(cursor..cursor + 6) {
            let vlen = u16::from_be_bytes([v_bytes[0], v_bytes[1]]) as usize;
            if vlen == 4 {
                let val = i32::from_be_bytes([v_bytes[2], v_bytes[3], v_bytes[4], v_bytes[5]]);
                levels.push(val.clamp(0, 100) as u8);
                cursor += 6;
            }
        }
        while let Some(hdr) = body.get(cursor..cursor + 7) {
            if hdr[0] == 0x21 && hdr[1] == 0 && hdr[2] == 0 && hdr[3] == 0 && hdr[4] == 4 {
                if let Some(vb) = body.get(cursor + 5..cursor + 9) {
                    let val = i32::from_be_bytes([vb[0], vb[1], vb[2], vb[3]]);
                    levels.push(val.clamp(0, 100) as u8);
                    cursor += 9;
                    continue;
                }
            }
            break;
        }
    }

    let channels = levels
        .into_iter()
        .enumerate()
        .map(|(idx, lvl)| InkChannelLevel {
            slot: (idx + 1) as u8,
            code: format!("C{}", idx + 1),
            name: tr!("Channel {number}", number = idx + 1),
            level_percent: lvl,
            swatch_rgb: Rgb8::new(90, 140, 200),
        })
        .collect();

    Ok(PrinterStatus {
        printer_uri: printer_uri.to_string(),
        online: true,
        state_summary: tr!("Online – IPP status received").to_string(),
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generic_ipp_marker_levels() {
        let mut fake_ipp = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x04];
        write_ipp_attr(&mut fake_ipp, 0x21, "marker-levels", &95i32.to_be_bytes());
        for lvl in [80i32, 65, 50] {
            write_ipp_attr(&mut fake_ipp, 0x21, "", &lvl.to_be_bytes());
        }
        fake_ipp.push(0x03);

        let status =
            parse_ipp_marker_response("ipp://localhost:631/printers/TestPrinter", &fake_ipp)
                .unwrap();
        assert!(status.online);
        assert_eq!(status.channels.len(), 4);
        assert_eq!(status.channels[0].level_percent, 95);
        assert_eq!(status.channels[3].level_percent, 50);
    }
}
