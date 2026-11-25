//! Command line: the viewer itself plus the agent-facing commands.
//!
//! Publishing commands talk to a running viewer over IPC. When no viewer is
//! running they write to the store directly, so nothing an agent publishes is
//! lost and it shows up the next time the pane is opened.

use std::{
    env, fs,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use clap::{Args, Parser, Subcommand};
use serde_json::{Map, Value, json};

use crate::{
    app,
    config::Config,
    error::{Error, Result},
    gui::{self, launch},
    protocol::{
        Action, ArtifactInput, ArtifactPatch, Request, Response,
        artifact::{ARTIFACT_TYPES, is_store_file_name},
    },
    storage::{self, Repository},
    terminal::{self, protocol_name},
    transport::{self, ipc},
};

#[derive(Parser)]
#[command(name = "herdr-artifacts", version, about = "Visual, interactive artifacts for Herdr agents, shown in a native window")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run the native window in this process (default when no command is given)
    Window,
    /// Run the viewer inside the terminal
    Tui,
    /// Publish an artifact: image, qr, message, markdown, table, chart, code, or json
    Publish(PublishArgs),
    /// Change an existing artifact
    Update(UpdateArgs),
    /// Delete an artifact
    Remove { id: String },
    /// List stored artifacts
    List {
        #[arg(long)]
        json: bool,
    },
    /// Print one artifact as JSON
    Show { id: String },
    /// Print user events (responses to actions)
    Events {
        /// Keep running and print new events as they happen
        #[arg(long)]
        follow: bool,
        /// Only events for this artifact
        #[arg(long)]
        artifact: Option<String>,
    },
    /// Block until the user responds to an artifact, then print the event
    Wait {
        id: String,
        /// Give up after this many seconds (exit code 2); 0 waits forever
        #[arg(long, default_value_t = 0)]
        timeout: u64,
    },
    /// Show paths, limits, IPC state and detected terminal capabilities
    Doctor,
    /// Open the viewer: a native window, or a Herdr terminal pane
    Open {
        /// `window` is a native window; the others are Herdr terminal panes
        #[arg(long, default_value = "window", value_parser = ["window", "split", "tab", "zoomed", "overlay"])]
        placement: String,
    },
}

#[derive(Args, Default)]
pub struct Common {
    /// Artifact id; publishing with an existing id replaces that artifact
    #[arg(long)]
    pub id: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    /// Defaults to $HERDR_ARTIFACTS_PROJECT or the current directory name
    #[arg(long)]
    pub project: Option<String>,
    /// Defaults to $HERDR_ARTIFACTS_AGENT
    #[arg(long)]
    pub agent: Option<String>,
    /// Short free-form state, e.g. running, done
    #[arg(long)]
    pub status: Option<String>,
    /// Action as id:label[:type[:url]]; append ! to the id to require confirmation
    #[arg(long = "action", value_name = "SPEC")]
    pub actions: Vec<String>,
}

#[derive(Args)]
pub struct PublishArgs {
    /// image | qr | message | markdown | table | chart | code | json
    pub kind: String,
    /// Source file (use - for stdin where text is expected)
    pub source: Option<PathBuf>,
    #[command(flatten)]
    pub common: Common,
    /// QR payload
    #[arg(long)]
    pub data: Option<String>,
    /// Text for message, markdown or code (instead of a file)
    #[arg(long)]
    pub body: Option<String>,
    /// Message level: info, success, warning, error
    #[arg(long)]
    pub level: Option<String>,
    /// Code language label
    #[arg(long)]
    pub language: Option<String>,
    /// Chart style: bar or line
    #[arg(long)]
    pub style: Option<String>,
    /// Chart unit suffix, e.g. ms
    #[arg(long)]
    pub unit: Option<String>,
    /// Open the native window, or bring it to the front
    #[arg(long)]
    pub show: bool,
    /// Chart point as label=value (repeatable)
    #[arg(long = "point", value_name = "LABEL=VALUE")]
    pub points: Vec<String>,
}

#[derive(Args)]
pub struct UpdateArgs {
    pub id: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
    /// New text for message, markdown or code
    #[arg(long)]
    pub body: Option<String>,
    /// New message level
    #[arg(long)]
    pub level: Option<String>,
    /// New QR payload
    #[arg(long)]
    pub data: Option<String>,
    /// Replace the image (image artifacts) or text (markdown/code) from a file
    #[arg(long)]
    pub source: Option<PathBuf>,
    /// Replace all actions (id:label[:type[:url]])
    #[arg(long = "action", value_name = "SPEC")]
    pub actions: Vec<String>,
    /// Remove all actions
    #[arg(long)]
    pub clear_actions: bool,
    /// Shallow-merge this JSON object into the content
    #[arg(long, value_name = "JSON")]
    pub content: Option<String>,
}

/// Entry point used by `main`.
pub fn run(cli: Cli) -> ExitCode {
    match dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error[{}]: {e}", e.code());
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<ExitCode> {
    let cfg = Config::load()?;
    match cli.command.unwrap_or(Cmd::Window) {
        Cmd::Window => gui::run(cfg)?,
        Cmd::Tui => app::run(cfg)?,
        Cmd::Publish(args) => {
            let show = args.show;
            let input = build_input(&cfg, args)?;
            let ingested = ingested_image(&input);
            let result = deliver(&cfg, Request::Publish { artifact: input });
            if result.is_err() {
                // Do not leave an orphan copy behind when the publish failed.
                if let Some(name) = ingested {
                    storage::discard_file(&cfg, &name);
                }
            }
            let id = result?.id;
            // A failure here (e.g. no display) does not undo the publish.
            if show && let Err(e) = launch::open(&cfg, id) {
                eprintln!("warning: published, but the window did not open: {e}");
            }
        }
        Cmd::Update(args) => {
            let (id, patch) = build_patch(&cfg, args)?;
            submit(&cfg, Request::Update { id, patch }).map(drop)?;
        }
        Cmd::Remove { id } => submit(&cfg, Request::Remove { id }).map(drop)?,
        Cmd::List { json } => list(&cfg, json)?,
        Cmd::Show { id } => {
            let artifact = Repository::open(cfg)?.load(&id)?;
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        Cmd::Events { follow, artifact } => events(&cfg, follow, artifact.as_deref())?,
        Cmd::Wait { id, timeout } => return wait(&cfg, &id, timeout),
        Cmd::Doctor => doctor(&cfg)?,
        Cmd::Open { placement } if placement == "window" => launch::open(&cfg, None)?,
        Cmd::Open { placement } => return open_pane(&placement),
    }
    Ok(ExitCode::SUCCESS)
}

/// What happened to a delivered request.
struct Delivery {
    /// `"viewer"` (over IPC) or `"store"` (no viewer was running).
    via: &'static str,
    id: Option<String>,
}

/// Deliver a request to the viewer, or apply it to the store when no viewer
/// is running. Prints one JSON line describing the result and returns how it
/// was delivered: `"viewer"` or `"store"`.
pub fn submit(cfg: &Config, request: Request) -> Result<&'static str> {
    deliver(cfg, request).map(|d| d.via)
}

/// Like [`submit`], also returning the affected artifact's id.
fn deliver(cfg: &Config, request: Request) -> Result<Delivery> {
    let (response, delivered) = match ipc::send(cfg, request.clone())? {
        Some(response) => (response, "viewer"),
        None => {
            let mut repo = Repository::open(cfg.clone())?;
            (transport::handle(&mut repo, request).0, "store")
        }
    };
    match response {
        Response::Ok { id, revision } => {
            println!("{}", json!({"ok": true, "id": id, "revision": revision, "delivered": delivered}));
            Ok(Delivery { via: delivered, id })
        }
        Response::Error { code, message } => Err(match code.as_str() {
            "not_found" => Error::NotFound(message.trim_start_matches("artifact not found: ").to_string()),
            "too_large" => Error::TooLarge(message.trim_start_matches("too large: ").to_string()),
            "unauthorized" => Error::Unauthorized,
            "io" => Error::Io(message.trim_start_matches("io error: ").to_string()),
            _ => Error::Invalid(message.trim_start_matches("invalid artifact: ").to_string()),
        }),
    }
}

fn ingested_image(input: &ArtifactInput) -> Option<String> {
    (input.kind == "image").then(|| input.content.get("source")?.as_str().map(str::to_string)).flatten()
}

/// Turn `publish` arguments into an artifact, reading and ingesting files.
pub fn build_input(cfg: &Config, args: PublishArgs) -> Result<ArtifactInput> {
    let limit = cfg.limits.max_payload_bytes as u64;
    let source = args.source.as_deref();
    let mut input = if args.kind == "json" {
        let raw = read_text(source.ok_or_else(|| Error::invalid("publish json needs a file (or - for stdin)"))?, limit)?;
        let mut input: ArtifactInput = serde_json::from_str(&raw)?;
        // A contract may reference an image by path; bring it into the store.
        if input.kind == "image" {
            let path = input.content.get("source").and_then(Value::as_str).unwrap_or_default().to_string();
            if !(is_store_file_name(&path) && cfg.files_dir().join(&path).is_file()) {
                input.content = json!({"source": storage::ingest_file(cfg, Path::new(&path))?});
            }
        }
        input
    } else {
        let text = |what: &str| -> Result<String> {
            match (&args.body, source) {
                (Some(body), _) => Ok(body.clone()),
                (None, Some(path)) => read_text(path, limit),
                (None, None) => Err(Error::Invalid(format!("{what} needs a file, - for stdin, or --body"))),
            }
        };
        let content = match args.kind.as_str() {
            "image" => {
                let path = source.ok_or_else(|| Error::invalid("publish image needs a file"))?;
                json!({"source": storage::ingest_file(cfg, path)?})
            }
            "qr" => json!({"data": args.data.clone().ok_or_else(|| Error::invalid("publish qr needs --data"))?}),
            "message" => {
                let mut c = json!({"body": text("publish message")?});
                if let Some(level) = &args.level {
                    c["level"] = json!(level);
                }
                c
            }
            "markdown" => json!({"text": text("publish markdown")?}),
            "code" => {
                let language = args.language.clone().or_else(|| {
                    source.and_then(|p| p.extension()).and_then(|e| e.to_str()).map(str::to_string)
                });
                json!({"text": text("publish code")?, "language": language})
            }
            "table" => {
                let path = source.ok_or_else(|| Error::invalid("publish table needs a .csv or .json file"))?;
                let raw = read_text(path, limit)?;
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
                    serde_json::from_str(&raw)?
                } else {
                    parse_csv(&raw)?
                }
            }
            "chart" => {
                let mut c = match source {
                    Some(path) => serde_json::from_str(&read_text(path, limit)?)?,
                    None => json!({"points": args.points.iter().map(|p| parse_point(p)).collect::<Result<Vec<_>>>()?}),
                };
                if let Some(style) = &args.style {
                    c["style"] = json!(style);
                }
                if let Some(unit) = &args.unit {
                    c["unit"] = json!(unit);
                }
                c
            }
            other => {
                return Err(Error::Invalid(format!("unknown type '{other}' (use {}, or json)", ARTIFACT_TYPES.join(", "))));
            }
        };
        let default_title = source
            .filter(|p| p.as_os_str() != "-")
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        ArtifactInput { kind: args.kind.clone(), title: default_title, content, ..Default::default() }
    };

    // Flags override whatever the JSON contract carried.
    let c = args.common;
    input.id = c.id.or(input.id);
    input.title = c.title.or(input.title);
    input.description = c.description.or(input.description);
    input.status = c.status.or(input.status);
    input.project_id = c.project.or(input.project_id).or_else(default_project);
    input.agent_id = c.agent.or(input.agent_id).or_else(|| env_value("HERDR_ARTIFACTS_AGENT"));
    if !c.actions.is_empty() {
        input.actions = c.actions.iter().map(|s| parse_action(s)).collect::<Result<_>>()?;
    }
    Ok(input)
}

pub fn build_patch(cfg: &Config, args: UpdateArgs) -> Result<(String, ArtifactPatch)> {
    let mut content = match &args.content {
        Some(raw) => serde_json::from_str::<Map<String, Value>>(raw)?,
        None => Map::new(),
    };
    // The text field is `body` for messages and `text` for markdown/code.
    let kind = if args.body.is_some() || args.source.is_some() {
        Repository::open(cfg.clone())?.load(&args.id)?.kind
    } else {
        String::new()
    };
    let text_key = if kind == "message" { "body" } else { "text" };
    if let Some(body) = args.body {
        content.insert(text_key.into(), json!(body));
    }
    if let Some(path) = &args.source {
        if kind == "image" {
            content.insert("source".into(), json!(storage::ingest_file(cfg, path)?));
        } else {
            content.insert(text_key.into(), json!(read_text(path, cfg.limits.max_payload_bytes as u64)?));
        }
    }
    if let Some(level) = args.level {
        content.insert("level".into(), json!(level));
    }
    if let Some(data) = args.data {
        content.insert("data".into(), json!(data));
    }
    let actions = if args.clear_actions {
        Some(Vec::new())
    } else if args.actions.is_empty() {
        None
    } else {
        Some(args.actions.iter().map(|s| parse_action(s)).collect::<Result<_>>()?)
    };
    let patch = ArtifactPatch {
        title: args.title,
        description: args.description,
        status: args.status,
        content: (!content.is_empty()).then_some(content),
        actions,
    };
    Ok((args.id, patch))
}

/// `id:label[:type[:url]]`, with `!` after the id to require confirmation.
pub fn parse_action(spec: &str) -> Result<Action> {
    let mut parts = spec.splitn(4, ':');
    let raw_id = parts.next().unwrap_or_default();
    let label = parts.next().ok_or_else(|| Error::Invalid(format!("action '{spec}' must be id:label[:type[:url]]")))?;
    let kind = parts.next().filter(|k| !k.is_empty()).unwrap_or("respond");
    let url = parts.next();
    let (id, confirm) = match raw_id.strip_suffix('!') {
        Some(id) => (id, true),
        None => (raw_id, false),
    };
    Ok(Action {
        id: id.to_string(),
        label: label.to_string(),
        kind: kind.to_string(),
        payload: url.map(|u| json!({"url": u})).unwrap_or_default(),
        confirm,
    })
}

fn parse_point(spec: &str) -> Result<Value> {
    let (label, value) = spec.rsplit_once('=').ok_or_else(|| Error::Invalid(format!("point '{spec}' must be label=value")))?;
    let value: f64 = value.trim().parse().map_err(|_| Error::Invalid(format!("point '{spec}': value is not a number")))?;
    Ok(json!({"label": label, "value": value}))
}

/// Minimal CSV: comma separated, double quotes for cells containing commas
/// or quotes (`""` escapes a quote). The first row is the header.
pub fn parse_csv(raw: &str) -> Result<Value> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let mut cells = Vec::new();
        let mut cell = String::new();
        let mut quoted = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                ('"', _) => quoted = !quoted,
                (',', false) => cells.push(std::mem::take(&mut cell)),
                (c, _) => cell.push(c),
            }
        }
        cells.push(cell);
        rows.push(cells.into_iter().map(|c| c.trim().to_string()).collect());
    }
    if rows.is_empty() {
        return Err(Error::invalid("csv is empty"));
    }
    let columns = rows.remove(0);
    Ok(json!({"columns": columns, "rows": rows}))
}

/// Read a text file (or stdin for `-`), refusing anything over `limit`.
fn read_text(path: &Path, limit: u64) -> Result<String> {
    let mut text = String::new();
    if path.as_os_str() == "-" {
        io::stdin().take(limit + 1).read_to_string(&mut text)?;
    } else {
        let meta = fs::metadata(path).map_err(|e| Error::Invalid(format!("cannot read {}: {e}", path.display())))?;
        if !meta.is_file() {
            return Err(Error::Invalid(format!("{} is not a regular file", path.display())));
        }
        if meta.len() > limit {
            return Err(Error::TooLarge(format!("{} is {} bytes, limit is {limit}", path.display(), meta.len())));
        }
        text = fs::read_to_string(path).map_err(|e| Error::Invalid(format!("cannot read {}: {e}", path.display())))?;
    }
    if text.len() as u64 > limit {
        return Err(Error::TooLarge(format!("input is larger than {limit} bytes")));
    }
    Ok(text)
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn default_project() -> Option<String> {
    env_value("HERDR_ARTIFACTS_PROJECT")
        .or_else(|| env::current_dir().ok()?.file_name().map(|n| n.to_string_lossy().into_owned()))
}

fn list(cfg: &Config, as_json: bool) -> Result<()> {
    let repo = Repository::open(cfg.clone())?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(repo.metas())?);
        return Ok(());
    }
    for m in repo.metas() {
        println!(
            "{:<20} {:<9} r{:<3} {:<12} {}  {}",
            m.id,
            m.kind,
            m.revision,
            m.agent_id.as_deref().unwrap_or("-"),
            m.created_at,
            m.title
        );
    }
    Ok(())
}

fn events(cfg: &Config, follow: bool, artifact: Option<&str>) -> Result<()> {
    let mut offset = 0;
    loop {
        let (batch, next) = storage::read_events(cfg, offset)?;
        offset = next;
        for event in batch.iter().filter(|e| artifact.is_none_or(|id| e.artifact_id == id)) {
            println!("{}", serde_json::to_string(event)?);
        }
        if !follow {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Wait for the user's answer to the artifact's current revision. Answers
/// given before `wait` started are found too, so there is no race between
/// publishing and waiting.
fn wait(cfg: &Config, id: &str, timeout: u64) -> Result<ExitCode> {
    let revision = Repository::open(cfg.clone())?.load(id)?.revision;
    let started = Instant::now();
    let mut offset = 0;
    loop {
        let (batch, next) = storage::read_events(cfg, offset)?;
        offset = next;
        if let Some(event) = batch.iter().find(|e| e.artifact_id == id && e.revision >= revision) {
            println!("{}", serde_json::to_string(event)?);
            return Ok(ExitCode::SUCCESS);
        }
        if timeout > 0 && started.elapsed() >= Duration::from_secs(timeout) {
            eprintln!("timeout: no response to '{id}' after {timeout}s");
            return Ok(ExitCode::from(2));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn doctor(cfg: &Config) -> Result<()> {
    let repo = Repository::open(cfg.clone())?;
    println!("herdr-artifacts {}", env!("CARGO_PKG_VERSION"));
    println!("home            {}", cfg.home.display());
    println!("artifacts       {}", repo.metas().len());
    println!("viewer (ipc)    {}", if ipc::is_running(cfg) { "running" } else { "not running (publishes go to the store)" });
    println!("limits          {:?}", cfg.limits);
    for var in ["TERM", "TERM_PROGRAM", "COLORTERM", "HERDR_ENV", "HERDR_PANE_ID", "HERDR_ARTIFACTS_IMAGE_PROTOCOL"] {
        println!("{var:<15} {}", env::var(var).unwrap_or_else(|_| "(unset)".into()));
    }
    if io::stdout().is_terminal() && io::stdin().is_terminal() {
        let (_, caps) = terminal::detect();
        println!("image protocol  {}{}", protocol_name(caps.protocol), if caps.forced { " (forced)" } else { "" });
        println!("cell size       {}x{} px{}", caps.font_size.0, caps.font_size.1, if caps.answered { "" } else { " (assumed; terminal did not answer)" });
        println!("truecolor       {}", caps.truecolor);
    } else {
        println!("image protocol  (not a terminal; run doctor inside the pane to detect)");
    }
    Ok(())
}

/// Open a terminal viewer pane through Herdr's own CLI.
fn open_pane(placement: &str) -> Result<ExitCode> {
    let herdr = env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into());
    let plugin = env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| "local.herdr-artifacts".into());
    // Pane ids are per platform in the manifest (ids must be unique).
    let entrypoint = if cfg!(windows) { "dashboard-windows" } else { "dashboard" };
    let mut cmd = std::process::Command::new(herdr);
    cmd.args(["plugin", "pane", "open", "--plugin", &plugin, "--entrypoint", entrypoint, "--placement", placement, "--focus"]);
    if placement == "split" {
        cmd.args(["--direction", "right"]);
    }
    let status = cmd.status().map_err(|e| Error::Io(format!("could not run herdr: {e}")))?;
    Ok(if status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_specs() {
        let a = parse_action("approve:Aprobar").unwrap();
        assert_eq!((a.id.as_str(), a.label.as_str(), a.kind.as_str(), a.confirm), ("approve", "Aprobar", "respond", false));
        let a = parse_action("reject!:Rechazar").unwrap();
        assert!(a.confirm && a.id == "reject");
        let a = parse_action("open:Abrir:open_url:http://localhost:8081/a?b=1").unwrap();
        assert_eq!(a.kind, "open_url");
        assert_eq!(a.url(), Some("http://localhost:8081/a?b=1"));
        assert!(parse_action("nolabel").is_err());
    }

    #[test]
    fn csv_parsing() {
        let v = parse_csv("name,size\n\"a,b\",1\n\"say \"\"hi\"\"\", 2\n\n").unwrap();
        assert_eq!(v, json!({"columns": ["name", "size"], "rows": [["a,b", "1"], ["say \"hi\"", "2"]]}));
        assert!(parse_csv("  \n").is_err());
    }

    #[test]
    fn points() {
        assert_eq!(parse_point("a=b=1.5").unwrap(), json!({"label": "a=b", "value": 1.5}));
        assert!(parse_point("a").is_err());
        assert!(parse_point("a=x").is_err());
    }
}
