use serde::Serialize;
use std::collections::HashMap;
use tokio::{io::AsyncWriteExt, sync::broadcast::Receiver};
use tracing::error;
use utrace_parser::stream_parser::TimestampedTracepoint;

#[derive(Serialize)]
enum EventType {
    #[serde(rename = "B")]
    SpanBegin,
    #[serde(rename = "E")]
    SpanEnd,
    #[serde(rename = "i")]
    Instant,
}

enum DrawingTypes {
    Span,
    Instant,
}

#[derive(Serialize)]
enum ArrowType {
    #[serde(rename = "s")]
    ArrowStart,
    #[serde(rename = "t")]
    ArrowStep,
}

#[derive(Serialize)]
struct Event {
    name: String,
    cat: String,
    #[serde(rename = "ph")]
    ty: EventType,
    pid: u32,
    tid: u32,
    ts: u64,
}

#[derive(Serialize)]
struct ArrowEvent {
    name: String,
    cat: String,
    #[serde(rename = "ph")]
    ty: ArrowType,
    pid: u32,
    tid: u32,
    ts: u64,
    id: u32,
    bp: String,
}

struct TraceEntry {
    last_timestamp: u64,
    unique_id: u32,
}

pub struct Store {
    hm: HashMap<u64, DrawingTypes>,
}

impl Store {
    pub fn new(tp_map: &HashMap<u8, utrace_core::trace_point::TracePointDataWithLocation>) -> Self {
        let mut hm = HashMap::new();

        for tp in tp_map.values() {
            let hash_id = tp.info.id;

            match tp.info.kind {
                utrace_core::trace_point::TracePointKind::AsyncEnter => (),
                utrace_core::trace_point::TracePointKind::AsyncExit => (),
                _ => {
                    hm.entry(hash_id)
                        .and_modify(|w| *w = DrawingTypes::Span)
                        .or_insert(DrawingTypes::Instant);
                }
            }
        }

        Store { hm }
    }

    pub async fn store<'a>(&self, fname: &str, mut chan: Receiver<TimestampedTracepoint<'a>>) {
        let mut events: HashMap<String, TraceEntry> = HashMap::new();
        let mut unique_id_counter: u32 = 0;

        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(fname)
            .await
        {
            file.write_all(b"[ \n").await;
            while let Ok(msg) = chan.recv().await {
                if let TimestampedTracepoint::Point {
                    timestamp: ts,
                    tracepoint: tp,
                } = msg
                {
                    let mut arrow: Option<ArrowEvent> = None;
                    let mut cat = "tmp".to_owned();
                    let name = tp.info.name.to_owned().unwrap();

                    let event_type: EventType = if tp.info.kind.is_enter() {
                        if events.contains_key(&name) {
                            let existing_event: &mut TraceEntry = events.get_mut(&name).unwrap();

                            arrow = Some(ArrowEvent {
                                name: name.clone(),
                                cat: name.clone(),
                                ty: ArrowType::ArrowStep,
                                pid: 1,
                                tid: 1,
                                ts,
                                id: existing_event.unique_id,
                                bp: "e".to_owned(),
                            });
                            // If the event already exists, update the timestamp
                            existing_event.last_timestamp = ts;
                        } else {
                            // New event, insert into the HashMap
                            match tp.info.kind {
                                utrace_core::trace_point::TracePointKind::AsyncEnter => {
                                    events.insert(
                                        name.clone(),
                                        TraceEntry {
                                            last_timestamp: ts,
                                            unique_id: unique_id_counter,
                                        },
                                    );
                                }
                                utrace_core::trace_point::TracePointKind::AsyncPollEnter => {
                                    events.insert(
                                        name.clone(),
                                        TraceEntry {
                                            last_timestamp: ts,
                                            unique_id: unique_id_counter,
                                        },
                                    );
                                }

                                _ => (),
                            }

                            arrow = Some(ArrowEvent {
                                name: name.clone(),
                                cat: name.clone(),
                                ty: ArrowType::ArrowStart,
                                pid: 1,
                                tid: 1,
                                ts,
                                id: unique_id_counter,
                                bp: "e".to_owned(),
                            });

                            unique_id_counter += 1;
                        }

                        match tp.info.kind {
                            utrace_core::trace_point::TracePointKind::AsyncEnter => {
                                EventType::Instant
                            }
                            utrace_core::trace_point::TracePointKind::AsyncExit => {
                                EventType::Instant
                            }
                            _ => match self.hm.get(&tp.info.id) {
                                Some(DrawingTypes::Span) => EventType::SpanBegin,
                                _ => EventType::Instant,
                            },
                        }
                    } else {
                        let end_id = if events.contains_key(&name) {
                            events.get_mut(&name).unwrap().last_timestamp = ts;
                            events.get_mut(&name).unwrap().unique_id
                        } else {
                            unique_id_counter
                        };

                        arrow = Some(ArrowEvent {
                            name: name.clone(),
                            cat: name.clone(),
                            ty: ArrowType::ArrowStep,
                            pid: 1,
                            tid: 1,
                            ts,
                            id: end_id,
                            bp: "e".to_owned(),
                        });

                        events.remove(&name);

                        match tp.info.kind {
                            utrace_core::trace_point::TracePointKind::AsyncEnter => {
                                EventType::Instant
                            }
                            utrace_core::trace_point::TracePointKind::AsyncExit => {
                                EventType::Instant
                            }
                            _ => match self.hm.get(&tp.info.id) {
                                Some(DrawingTypes::Span) => EventType::SpanEnd,
                                _ => EventType::Instant,
                            },
                        }
                    };

                    let msg_out = Event {
                        name,
                        cat: tp.info.kind.to_string(),
                        ty: event_type,
                        pid: 1,
                        tid: 1,
                        ts,
                    };
                    file.write_all(serde_json::to_string(&msg_out).unwrap().as_bytes())
                        .await;
                    file.write_all(",\n".as_bytes()).await;

                    if let Some(arrow_event) = arrow {
                        file.write_all(serde_json::to_string(&arrow_event).unwrap().as_bytes())
                            .await;
                        file.write_all(",\n".as_bytes()).await;
                    }
                }
            }
            file.write_all(b"]").await; // Properly close the JSON array
        } else {
            error!("Cannot open file {fname} for writing");
        }
    }
}
