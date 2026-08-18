use std::convert::Infallible;

use eventsource_stream::Eventsource;
use futures_util::{StreamExt, stream};
use serde_json::Value;
use thiserror::Error;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
const SSE_FEED_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SseEvent {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DecodedSse {
    pub events: Vec<SseEvent>,
    pub trailing_incomplete_frame: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DecodedNdjson {
    pub items: Vec<NdjsonItem>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum NdjsonItem {
    Record(Value),
    Error(FramingError),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum FramingError {
    #[error("invalid SSE framing: {message}")]
    InvalidSse { message: String },
    #[error("invalid NDJSON object on line {line}: {message}")]
    InvalidNdjson { line: usize, message: String },
    #[error("incomplete NDJSON object on line {line}: {message}")]
    IncompleteNdjson { line: usize, message: String },
    #[error("NDJSON line {line} must contain a JSON object")]
    NdjsonNotObject { line: usize },
}

pub(crate) async fn decode_sse_chunks(chunks: Vec<Vec<u8>>) -> Result<DecodedSse, FramingError> {
    let chunks = strip_initial_utf8_bom(chunks);
    let raw = into_owned_sse_bytes(chunks);
    if raw.starts_with(UTF8_BOM) {
        return Err(FramingError::InvalidSse {
            message: "multiple UTF-8 byte-order marks at SSE stream start".into(),
        });
    }
    std::str::from_utf8(&raw).map_err(|error| FramingError::InvalidSse {
        message: error.to_string(),
    })?;
    let trailing_incomplete_frame = has_undispatched_sse_frame(&raw);
    let byte_stream = stream::iter(sse_feed_chunks(&raw).map(Ok::<_, Infallible>));
    let mut event_stream = byte_stream.eventsource();
    let mut events = Vec::new();

    while let Some(event) = event_stream.next().await {
        let event = event.map_err(|error| FramingError::InvalidSse {
            message: error.to_string(),
        })?;
        events.push(SseEvent {
            event: event.event,
            data: event.data,
        });
    }

    Ok(DecodedSse {
        events,
        trailing_incomplete_frame,
    })
}

pub(crate) fn decode_ndjson_chunks(chunks: Vec<Vec<u8>>) -> Result<Vec<Value>, FramingError> {
    let decoded = decode_ndjson_chunks_partial(chunks);
    let mut records = Vec::new();
    for item in decoded.items {
        match item {
            NdjsonItem::Record(record) => records.push(record),
            NdjsonItem::Error(error) => return Err(error),
        }
    }
    Ok(records)
}

pub(crate) fn decode_ndjson_chunks_partial(chunks: Vec<Vec<u8>>) -> DecodedNdjson {
    let bytes = chunks.concat();
    let mut items = Vec::new();
    let mut line_start = 0;
    let mut line_number = 1;

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }

        let line = strip_carriage_return(&bytes[line_start..index]);
        if !is_blank_line(line) {
            items.push(match parse_ndjson_object(line, line_number, false) {
                Ok(value) => NdjsonItem::Record(value),
                Err(error) => NdjsonItem::Error(error),
            });
        }
        line_start = index + 1;
        line_number += 1;
    }

    if line_start < bytes.len() {
        let tail = strip_carriage_return(&bytes[line_start..]);
        if !is_blank_line(tail) {
            items.push(match parse_ndjson_object(tail, line_number, true) {
                Ok(value) => NdjsonItem::Record(value),
                Err(error) => NdjsonItem::Error(error),
            });
        }
    }

    DecodedNdjson { items }
}

fn strip_initial_utf8_bom(mut chunks: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let mut prefix = [0; 3];
    let mut prefix_len = 0;
    for byte in chunks.iter().flatten().take(UTF8_BOM.len()) {
        prefix[prefix_len] = *byte;
        prefix_len += 1;
    }

    if prefix_len == UTF8_BOM.len() && prefix.as_slice() == UTF8_BOM {
        let mut remaining = UTF8_BOM.len();
        for chunk in &mut chunks {
            let strip_len = remaining.min(chunk.len());
            chunk.drain(..strip_len);
            remaining -= strip_len;
            if remaining == 0 {
                break;
            }
        }
    }

    chunks.retain(|chunk| !chunk.is_empty());
    chunks
}

fn into_owned_sse_bytes(mut chunks: Vec<Vec<u8>>) -> Vec<u8> {
    if chunks.len() == 1 {
        return chunks.pop().expect("one chunk is present");
    }

    let capacity = chunks.iter().map(Vec::len).sum();
    let mut raw = Vec::with_capacity(capacity);
    for chunk in chunks {
        raw.extend_from_slice(&chunk);
    }
    raw
}

fn sse_feed_chunks(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    raw.chunks(SSE_FEED_CHUNK_BYTES)
}

fn has_undispatched_sse_frame(bytes: &[u8]) -> bool {
    let mut frame_has_content = false;
    let mut line_start = 0;
    let mut index = 0;

    while index < bytes.len() {
        if !matches!(bytes[index], b'\r' | b'\n') {
            index += 1;
            continue;
        }

        frame_has_content = index != line_start;

        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
        }
        index += 1;
        line_start = index;
    }

    frame_has_content || line_start < bytes.len()
}

fn strip_carriage_return(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn is_blank_line(line: &[u8]) -> bool {
    line.iter().all(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
}

fn parse_ndjson_object(
    line: &[u8],
    line_number: usize,
    is_final_tail: bool,
) -> Result<Value, FramingError> {
    match serde_json::from_slice::<Value>(line) {
        Ok(value) if value.is_object() => Ok(value),
        Ok(_) => Err(FramingError::NdjsonNotObject { line: line_number }),
        Err(error) if is_final_tail && error.is_eof() => Err(FramingError::IncompleteNdjson {
            line: line_number,
            message: error.to_string(),
        }),
        Err(error) => Err(FramingError::InvalidNdjson {
            line: line_number,
            message: error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        FramingError, NdjsonItem, SSE_FEED_CHUNK_BYTES, SseEvent, decode_ndjson_chunks,
        decode_ndjson_chunks_partial, decode_sse_chunks, into_owned_sse_bytes, sse_feed_chunks,
    };

    fn bytes(value: &str) -> Vec<u8> {
        value.as_bytes().to_vec()
    }

    #[tokio::test]
    async fn sse_accepts_lf_and_crlf_boundaries() {
        let lf = decode_sse_chunks(vec![bytes("event: message\ndata: {\"ok\":true}\n\n")])
            .await
            .expect("LF-framed SSE should decode");
        let crlf = decode_sse_chunks(vec![bytes("event: message\r\ndata: {\"ok\":true}\r\n\r\n")])
            .await
            .expect("CRLF-framed SSE should decode");

        assert_eq!(lf, crlf);
        assert_eq!(
            lf.events,
            vec![SseEvent {
                event: "message".into(),
                data: "{\"ok\":true}".into(),
            }]
        );
        assert!(!lf.trailing_incomplete_frame);
    }

    #[tokio::test]
    async fn sse_joins_multiline_data_and_preserves_event_name() {
        let decoded = decode_sse_chunks(vec![bytes(
            "event: response.output_text.delta\ndata: first\ndata: second\n\n",
        )])
        .await
        .expect("multiline SSE should decode");

        assert_eq!(
            decoded.events,
            vec![SseEvent {
                event: "response.output_text.delta".into(),
                data: "first\nsecond".into(),
            }]
        );
        assert!(!decoded.trailing_incomplete_frame);
    }

    #[tokio::test]
    async fn sse_ignores_comments_and_decodes_extension_events() {
        let decoded = decode_sse_chunks(vec![bytes(
            ": keepalive\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\n",
        )])
        .await
        .expect("comments and extension events should decode");

        assert_eq!(
            decoded.events,
            vec![SseEvent {
                event: "ping".into(),
                data: "{\"type\":\"ping\"}".into(),
            }]
        );
        assert!(!decoded.trailing_incomplete_frame);
    }

    #[tokio::test]
    async fn sse_is_invariant_across_every_byte_split() {
        let large_input = vec![b'x'; 760 * 1024 + 17];
        let allocation = large_input.as_ptr();
        let large_input = into_owned_sse_bytes(vec![large_input]);
        assert_eq!(large_input.as_ptr(), allocation);
        let feed_chunks = sse_feed_chunks(&large_input).collect::<Vec<_>>();
        assert!(feed_chunks.len() > 1);
        assert!(
            feed_chunks
                .iter()
                .all(|chunk| chunk.len() <= SSE_FEED_CHUNK_BYTES)
        );
        assert_eq!(feed_chunks.concat(), large_input);

        let fixture = bytes(
            "\u{feff}: keepalive\r\nevent: response.created\r\ndata:{\"type\":\"response.created\",\"text\":\"你好🙂\"}\r\n\r\nevent: response.output_text.delta\ndata: first\ndata: second\n\n",
        );
        let expected = decode_sse_chunks(vec![fixture.clone()])
            .await
            .expect("one-chunk fixture should decode");

        for split in 0..=fixture.len() {
            let actual =
                decode_sse_chunks(vec![fixture[..split].to_vec(), fixture[split..].to_vec()])
                    .await
                    .unwrap_or_else(|error| panic!("split {split} failed: {error}"));
            assert_eq!(actual, expected, "split offset {split}");
        }

        let double_bom = bytes("\u{feff}\u{feff}data: {\"ok\":true}\n\n");
        let cases = [
            ("one chunk", vec![double_bom.clone()]),
            (
                "BOMs split around empty chunks",
                vec![
                    vec![0xef],
                    vec![],
                    vec![0xbb, 0xbf],
                    vec![0xef, 0xbb],
                    vec![],
                    vec![0xbf],
                    bytes("data: {\"ok\":true}\n\n"),
                ],
            ),
            (
                "one byte per chunk",
                double_bom.iter().map(|byte| vec![*byte]).collect(),
            ),
        ];
        let expected_error = Err(FramingError::InvalidSse {
            message: "multiple UTF-8 byte-order marks at SSE stream start".into(),
        });

        for (case, chunks) in cases {
            assert_eq!(decode_sse_chunks(chunks).await, expected_error, "{case}");
        }
    }

    #[tokio::test]
    async fn sse_reports_trailing_incomplete_frame() {
        let decoded = decode_sse_chunks(vec![bytes(
            "event: response.output_text.delta\ndata: {\"delta\":\"partial\"}",
        )])
        .await
        .expect("a trailing partial SSE frame is reported, not a decoder error");

        assert!(decoded.events.is_empty());
        assert!(decoded.trailing_incomplete_frame);
    }

    #[test]
    fn ndjson_is_invariant_across_every_byte_split() {
        let fixture =
            bytes("{\"message\":{\"content\":\"你好🙂\"},\"done\":false}\r\n\n{\"done\":true}\n");
        let expected =
            decode_ndjson_chunks(vec![fixture.clone()]).expect("one-chunk fixture should decode");

        for split in 0..=fixture.len() {
            let actual =
                decode_ndjson_chunks(vec![fixture[..split].to_vec(), fixture[split..].to_vec()])
                    .unwrap_or_else(|error| panic!("split {split} failed: {error}"));
            assert_eq!(actual, expected, "split offset {split}");
        }
    }

    #[test]
    fn ndjson_accepts_a_complete_final_object_without_newline() {
        let decoded = decode_ndjson_chunks(vec![bytes(
            "\n{\"message\":{\"content\":\"hello\"},\"done\":false}\n\n{\"done\":true}",
        )])
        .expect("a complete final JSON object should decode without a newline");

        assert_eq!(
            decoded,
            vec![
                json!({"message": {"content": "hello"}, "done": false}),
                json!({"done": true}),
            ]
        );
    }

    #[test]
    fn ndjson_rejects_a_partial_final_object() {
        let result = decode_ndjson_chunks(vec![bytes(
            "{\"message\":{\"content\":\"hello\"},\"done\":false}\n{\"done\":",
        )]);

        assert!(matches!(
            result,
            Err(FramingError::IncompleteNdjson { line: 2, .. })
        ));

        let truncated_utf8 = decode_ndjson_chunks(vec![
            b"{\"message\":{\"content\":\"".to_vec(),
            vec![0xf0, 0x9f],
        ]);
        assert!(matches!(
            truncated_utf8,
            Err(FramingError::IncompleteNdjson { line: 1, .. })
        ));

        for non_object in ["[]", "42", "null"] {
            assert_eq!(
                decode_ndjson_chunks(vec![bytes(non_object)]),
                Err(FramingError::NdjsonNotObject { line: 1 })
            );
        }
    }

    #[test]
    fn ndjson_partial_decode_preserves_records_and_errors_in_wire_order() {
        let body = bytes(
            "{\"record\":0}\nPRIVATE_BAD_ONE\n{\"record\":1}\n[]\n{\"record\":2}\n{\"tail\":",
        );
        let decoded = decode_ndjson_chunks_partial(vec![body.clone()]);

        assert_eq!(decoded.items.len(), 6);
        assert!(matches!(
            &decoded.items[0],
            NdjsonItem::Record(value) if value == &json!({"record": 0})
        ));
        assert!(matches!(
            &decoded.items[1],
            NdjsonItem::Error(FramingError::InvalidNdjson { line: 2, .. })
        ));
        assert!(matches!(
            &decoded.items[2],
            NdjsonItem::Record(value) if value == &json!({"record": 1})
        ));
        assert!(matches!(
            &decoded.items[3],
            NdjsonItem::Error(FramingError::NdjsonNotObject { line: 4 })
        ));
        assert!(matches!(
            &decoded.items[4],
            NdjsonItem::Record(value) if value == &json!({"record": 2})
        ));
        assert!(matches!(
            &decoded.items[5],
            NdjsonItem::Error(FramingError::IncompleteNdjson { line: 6, .. })
        ));

        assert!(matches!(
            decode_ndjson_chunks(vec![body]),
            Err(FramingError::InvalidNdjson { line: 2, .. })
        ));
    }
}
