use crate::{error, McpServer};
use serde_json::Value;
use std::io::{self, BufRead, Write};

pub const MAX_FRAME_BYTES: usize = 1_048_576;

enum Frame {
    Data(Vec<u8>),
    TooLarge,
}

/// Drain oversized lines with bounded memory, then continue on the next frame.
fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Frame>> {
    let mut frame = Vec::new();
    let mut oversized = false;
    loop {
        let (consumed, delimiter, eof) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                (0, false, true)
            } else {
                let end = available.iter().position(|byte| *byte == b'\n');
                let length = end.unwrap_or(available.len());
                if !oversized {
                    if frame.len().saturating_add(length) > MAX_FRAME_BYTES {
                        oversized = true;
                        frame.clear();
                    } else {
                        frame.extend_from_slice(&available[..length]);
                    }
                }
                (length + usize::from(end.is_some()), end.is_some(), false)
            }
        };
        reader.consume(consumed);
        if eof {
            return Ok(if oversized {
                Some(Frame::TooLarge)
            } else if frame.is_empty() {
                None
            } else {
                Some(Frame::Data(frame))
            });
        }
        if delimiter {
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            return Ok(Some(if oversized {
                Frame::TooLarge
            } else {
                Frame::Data(frame)
            }));
        }
    }
}

pub fn run(
    server: &mut McpServer,
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    while let Some(frame) = read_frame(&mut input)? {
        let response = match frame {
            Frame::Data(data) => server.handle_frame(&data),
            Frame::TooLarge => Some(error(
                Value::Null,
                -32600,
                "Frame excede o limite de 1 MiB.",
            )),
        };
        if let Some(response) = response {
            let mut serialized = serde_json::to_vec(&response)?;
            if serialized.len() > MAX_FRAME_BYTES {
                serialized = serde_json::to_vec(&error(
                    response.get("id").cloned().unwrap_or(Value::Null),
                    -32603,
                    "Resposta excede 1 MiB; reduza o contexto ou a projeção solicitada.",
                ))?;
            }
            output.write_all(&serialized)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}
