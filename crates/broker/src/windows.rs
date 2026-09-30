use ime_broker::Broker;
#[path = "faults.rs"]
mod faults;
use ime_broker_transport::{client::Pipe, security::*};
use ime_protocol::{
    wire::*, BrokerGeneration, ClientInstanceId, Epoch, MessageIdentity, RequestSeq, SessionId,
};
use ime_rime_adapter::NativeRime;
use std::{
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    sync::{mpsc, oneshot, Semaphore},
};

enum Job {
    Connect(ClientInstanceId, oneshot::Sender<Result<Hello, String>>),
    Request(ClientInstanceId, Request, oneshot::Sender<Reply>),
    Disconnect(ClientInstanceId),
}
const DEADLINE: Duration = Duration::from_secs(5);

pub fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!("ime-broker [--runtime DIR] [--user-data DIR] [--deploy] [--ready-file FILE]\nime-broker --pipe-smoke [TEXT]\n--dev-client EXE may be repeated. Technical beta: same logon and explicit executable paths; learning disabled.");
        return Ok(());
    }
    let context = PeerContext::current()?;
    if let Some(index) = args.iter().position(|a| a == "--pipe-smoke") {
        return diagnostic(
            &context,
            args.get(index + 1).map(String::as_str).unwrap_or("nihao "),
        );
    }
    let runtime = match args.iter().position(|a| a == "--runtime") {
        Some(index) => PathBuf::from(args.get(index + 1).ok_or("missing --runtime directory")?),
        None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/rime-runtime"),
    };
    let deploy = args.iter().any(|a| a == "--deploy");
    let user_data = match args.iter().position(|a| a == "--user-data") {
        Some(index) => PathBuf::from(args.get(index + 1).ok_or("missing --user-data directory")?),
        None => runtime.join("user"),
    };
    let ready_file = match args.iter().position(|a| a == "--ready-file") {
        Some(index) => Some(PathBuf::from(
            args.get(index + 1).ok_or("missing --ready-file path")?,
        )),
        None => None,
    };
    let faults = Arc::new(faults::Faults::from_args(&args)?);
    let mut allowed = Vec::new();
    for index in args
        .iter()
        .enumerate()
        .filter_map(|(i, a)| (a == "--dev-client").then_some(i))
    {
        let path = std::fs::canonicalize(args.get(index + 1).ok_or("missing --dev-client path")?)
            .map_err(|e| e.to_string())?;
        allowed.push(
            path.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_lowercase(),
        );
    }
    let (sender, mut receiver) = mpsc::channel::<Job>(32);
    let (ready, startup) = std::sync::mpsc::sync_channel(1);
    let generation = BrokerGeneration(
        (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos() as u64)
            ^ u64::from(std::process::id()),
    );
    let worker = std::thread::Builder::new()
        .name("wufan-rime-serial".into())
        .spawn(move || {
            let engine = NativeRime::start(
                &runtime.join("native/dist/lib/rime.dll"),
                &runtime.join("shared"),
                &user_data,
                deploy,
            );
            let mut broker = match engine {
                Ok(engine) => {
                    let _ = ready.send(Ok(()));
                    Broker::new(engine, generation)
                }
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            while let Some(job) = receiver.blocking_recv() {
                match job {
                    Job::Connect(client, reply) => {
                        let _ = reply.send(broker.connect(client));
                    }
                    Job::Request(client, request, reply) => {
                        let _ = reply.send(broker.handle(client, request));
                    }
                    Job::Disconnect(client) => broker.disconnect(client),
                }
            }
        })
        .map_err(|e| e.to_string())?;
    startup.recv().map_err(|e| e.to_string())??;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let result = rt.block_on(serve(context, sender, allowed, faults, ready_file));
    drop(rt);
    worker
        .join()
        .map_err(|_| "librime worker panicked".to_string())?;
    result
}

fn listener(name: &str, security: &PipeSecurity, first: bool) -> Result<NamedPipeServer, String> {
    let mut attributes = security.attributes();
    // Borrowed descriptor remains live through synchronous CreateNamedPipeW.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(10)
            .in_buffer_size(MAX_FRAME as u32)
            .out_buffer_size(MAX_FRAME as u32)
            .create_with_security_attributes_raw(name, std::ptr::from_mut(&mut attributes).cast())
            .map_err(|e| e.to_string())
    }
}
async fn serve(
    context: PeerContext,
    sender: mpsc::Sender<Job>,
    allowed: Vec<String>,
    faults: Arc<faults::Faults>,
    ready_file: Option<PathBuf>,
) -> Result<(), String> {
    let name = context.pipe_name();
    let security = PipeSecurity::for_logon(&context)?;
    let mut pipe = listener(&name, &security, true)?;
    // Publish only after deployment and exclusive pipe ownership succeeded. The launcher
    // supplies a fresh private path, so a previous process cannot leave a stale readiness signal.
    if let Some(path) = ready_file {
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, std::process::id().to_string()).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())?;
    }
    println!("Broker ready: {name}\nlibrime 1.17.0 / wufan_pinyin / learning disabled");
    let capacity = Arc::new(Semaphore::new(8));
    let mut next_client = 1u64;
    loop {
        pipe.connect().await.map_err(|e| e.to_string())?;
        let connected = pipe;
        pipe = listener(&name, &security, false)?;
        if authorize_development_client(&connected, &context, &allowed).is_err() {
            continue;
        }
        let Ok(permit) = capacity.clone().try_acquire_owned() else {
            continue;
        };
        let client = ClientInstanceId(next_client);
        next_client = next_client
            .checked_add(1)
            .ok_or("client incarnation exhausted")?;
        let sender = sender.clone();
        let faults = faults.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = connection(connected, client, &sender, &faults).await {
                eprintln!("connection {client} closed: {error}");
            }
            let _ = sender.send(Job::Disconnect(client)).await;
        });
    }
}
async fn connection(
    mut pipe: NamedPipeServer,
    client: ClientInstanceId,
    sender: &mpsc::Sender<Job>,
    faults: &faults::Faults,
) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    sender
        .try_send(Job::Connect(client, tx))
        .map_err(|e| e.to_string())?;
    let hello = tokio::time::timeout(DEADLINE, rx)
        .await
        .map_err(|_| "engine deadline")?
        .map_err(|e| e.to_string())??;
    write(&mut pipe, &hello).await?;
    loop {
        let mut header = [0; 4];
        tokio::time::timeout(Duration::from_secs(60), pipe.read_exact(&mut header))
            .await
            .map_err(|_| "idle deadline")?
            .map_err(|e| e.to_string())?;
        let mut body = vec![0; frame_len(header)?];
        tokio::time::timeout(DEADLINE, pipe.read_exact(&mut body))
            .await
            .map_err(|_| "frame deadline")?
            .map_err(|e| e.to_string())?;
        let request: Request = decode(&body)?;
        let (tx, rx) = oneshot::channel();
        sender
            .try_send(Job::Request(client, request.clone(), tx))
            .map_err(|e| e.to_string())?;
        let reply = tokio::time::timeout(DEADLINE, rx)
            .await
            .map_err(|_| "engine deadline")?
            .map_err(|e| e.to_string())?;
        faults.before_reply(&request, &reply).await?;
        write(&mut pipe, &reply).await?;
    }
}
async fn write(pipe: &mut NamedPipeServer, value: &impl serde::Serialize) -> Result<(), String> {
    tokio::time::timeout(DEADLINE, pipe.write_all(&encode(value)?))
        .await
        .map_err(|_| "write deadline")?
        .map_err(|e| e.to_string())
}

fn diagnostic(context: &PeerContext, input: &str) -> Result<(), String> {
    if input.len() > 256 || !input.is_ascii() {
        return Err("diagnostic input must be at most 256 ASCII bytes".into());
    }
    // Exercise the same deadline-bound overlapped transport as the TSF worker.
    let pipe = Pipe::open(&context.pipe_name())?;
    authorize_server(&pipe, context)?;
    let hello: Hello = pipe.receive(&std::sync::atomic::AtomicBool::new(false))?;
    if hello.version != VERSION || hello.learning_enabled {
        return Err("incompatible Broker".into());
    }
    let mut identity = MessageIdentity {
        client_instance_id: hello.client_instance_id,
        broker_generation: hello.broker_generation,
        session_id: SessionId(1),
        focus_epoch: Epoch(1),
        composition_epoch: Epoch(1),
        request_seq: RequestSeq(1),
    };
    exchange(&pipe, identity, Action::Open)?;
    for byte in input.bytes() {
        identity.request_seq.0 += 1;
        let reply = exchange(
            &pipe,
            identity,
            Action::Key {
                code: i32::from(byte),
                modifiers: 0,
            },
        )?;
        if let ReplyResult::View {
            preedit,
            candidates,
            commit,
            ..
        } = reply.result
        {
            println!(
                "preedit={preedit:?}, candidates={:?}",
                candidates.iter().map(|c| &c.text).collect::<Vec<_>>()
            );
            if let Some(commit) = commit {
                println!("commit={}", commit.text);
                std::io::stdout().flush().map_err(|e| e.to_string())?;
                identity.request_seq.0 += 1;
                exchange(
                    &pipe,
                    identity,
                    Action::CommitAck {
                        commit_id: commit.id,
                        outcome: CommitOutcome::Applied,
                    },
                )?;
            }
        }
    }
    identity.request_seq.0 += 1;
    exchange(&pipe, identity, Action::Close)?;
    Ok(())
}
fn exchange(pipe: &Pipe, identity: MessageIdentity, action: Action) -> Result<Reply, String> {
    let stopped = std::sync::atomic::AtomicBool::new(false);
    pipe.send(
        &Request {
            version: VERSION,
            identity,
            action,
        },
        &stopped,
    )?;
    let reply: Reply = pipe.receive(&stopped)?;
    if reply.version != VERSION || reply.identity != identity {
        return Err("stale Broker reply".into());
    }
    if let ReplyResult::Error { code } = reply.result {
        return Err(format!("Broker rejected request: {code:?}"));
    }
    Ok(reply)
}
