use anyhow::Result;
use chrometracing::Store;
use clap::Parser;
use serde_json::StreamDeserializer;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::sync::broadcast::{channel, Receiver, Sender};
use tracing::error;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use utrace_core::trace_point::{TracePointDataWithLocation, TracePointId};
use utrace_parser::stream_parser::TimestampedTracepoint;

pub mod chrometracing;
#[derive(Parser, Debug)]
struct Args {
    elf: PathBuf,

    #[arg(short, long)]
    tcp: Option<String>,

    #[arg(short, long)]
    chrometracing: Option<String>,

    #[arg(short, long)]
    stdout: bool,
}

async fn net_reader<'a>(
    addr: impl ToSocketAddrs,
    chan: Sender<TimestampedTracepoint<'a>>,
    id_mapping: &'a HashMap<TracePointId, TracePointDataWithLocation>,
) {
    if let Ok(mut socket) = TcpStream::connect(addr).await {
        let mut sd = utrace_parser::stream_parser::StreamParser::new(id_mapping);
        let mut buf = [0u8; 16536];

        while let Ok(size) = socket.read(&mut buf).await {
            for p in sd.push_and_parse(&buf[..size]) {
                chan.send(p);
            }
        }
    } else {
        error!("Unable to connect to requested address.");
    }
}

async fn tp_consumer<'a>(mut chan: Receiver<TimestampedTracepoint<'a>>) {
    while let Ok(p) = chan.recv().await {
        println!("{:?}", p);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    tracing_subscriber::fmt().init();

    let tp_data: HashMap<u8, TracePointDataWithLocation> =
        utrace_parser::elf_parser::parse(args.elf)?;

    let store_trace = Store::new(&tp_data);

    async_scoped::TokioScope::scope_and_block(|s| {
        let (tptx, tprx) = channel(1024);
        s.spawn(net_reader(args.tcp.unwrap(), tptx, &tp_data));

        if args.stdout {
            s.spawn(tp_consumer(tprx.resubscribe()));
        }
        if let Some(ref ct_file) = args.chrometracing {
            s.spawn(store_trace.store(ct_file, tprx.resubscribe()));
        }
        drop(tprx);
    });

    Ok(())
}
