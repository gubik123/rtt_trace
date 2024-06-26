use anyhow::{bail, Result};
use chrometracing::Store;
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::broadcast::{channel, Receiver, Sender};
use tracing::error;
use utrace_core::trace_point::{TracePointDataWithLocation, TracePointId};
use utrace_parser::stream_parser::TimestampedTracepoint;

const EVENT_QUEUE_LENGTH: usize = 1024;

pub mod chrometracing;
#[derive(Parser, Debug)]
struct Args {
    elf: PathBuf,

    #[arg(short = 's', long = "tcp-server", value_name = "BIND_ADDR")]
    tcp_server: Option<String>,

    #[arg(short = 't', long = "tcp", value_name = "ADDR")]
    tcp: Option<String>,

    #[arg(short = 'p', long = "stdin")]
    stdin: bool,

    #[arg(short = 'j', long = "out-ct", value_name = "LOG_PREFIX")]
    chrometracing: Option<String>,

    #[arg(short = 'c', long)]
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

        while let Ok(read) = socket.read(&mut buf).await {
            for p in sd.push_and_parse(&buf[..read]) {
                chan.send(p).expect("Event queue overflow");
            }
        }
    } else {
        error!("Unable to connect to requested address.");
    }
}

async fn net_server_reader<'a>(
    addr: impl ToSocketAddrs,
    chan: Sender<TimestampedTracepoint<'a>>,
    id_mapping: &'a HashMap<TracePointId, TracePointDataWithLocation>,
) {
    let l = TcpListener::bind(addr).await;

    if let Err(e) = l {
        error!("Unable to bind tcp socket. Error: {}", e);
        return;
    }
    let l = l.unwrap();

    while let Ok((mut socket, _)) = l.accept().await {
        let mut sd = utrace_parser::stream_parser::StreamParser::new(id_mapping);
        let mut buf = [0u8; 16536];

        while let Ok(read) = socket.read(&mut buf).await {
            for p in sd.push_and_parse(&buf[..read]) {
                chan.send(p).expect("Event queue overflow");
            }
        }
    }
    error!("Network error.");
}

async fn stdin_reader<'a>(
    chan: Sender<TimestampedTracepoint<'a>>,
    id_mapping: &'a HashMap<TracePointId, TracePointDataWithLocation>,
) {
    let mut sd = utrace_parser::stream_parser::StreamParser::new(id_mapping);
    let mut buf = [0u8; 16536];
    let mut stdin = tokio::io::stdin();

    while let Ok(read) = stdin.read(&mut buf).await {
        for p in sd.push_and_parse(&buf[..read]) {
            chan.send(p).expect("Event queue overflow");
        }
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

    let sources =
        args.tcp.is_some() as usize + args.tcp_server.is_some() as usize + args.stdin as usize;

    if sources > 1 {
        bail!("--tcp and --tcp-server and --stdin are mutually exclusive");
    }

    if sources < 1 {
        bail!("Stream source is not specified");
    }

    let tp_data: HashMap<u8, TracePointDataWithLocation> =
        utrace_parser::elf_parser::parse(args.elf)?;

    let store_trace = Store::new(&tp_data);

    async_scoped::TokioScope::scope_and_block(|s| {
        let (tptx, tprx) = channel(EVENT_QUEUE_LENGTH);
        if let Some(addr) = args.tcp {
            s.spawn(net_reader(addr, tptx, &tp_data));
        } else if let Some(addr) = args.tcp_server {
            s.spawn(net_server_reader(addr, tptx, &tp_data));
        } else if args.stdin {
            s.spawn(stdin_reader(tptx, &tp_data));
        }

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
