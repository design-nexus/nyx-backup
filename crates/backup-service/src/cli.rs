use anyhow::{Context, Result, ensure};
use backup_core::*;
use backup_system::*;
use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf};
#[derive(Parser)]
#[command(
    version,
    about = "Nyx Backup administration and versioned service client"
)]
struct Args {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}
#[derive(Subcommand)]
enum Cmd {
    Capabilities,
    Status,
    Inventory,
    Jobs,
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    Profiles,
    Destinations,
    Preview {
        profile: String,
    },
    Backup {
        profile: String,
        #[arg(long)]
        wait: bool,
    },
    Verify {
        profile: String,
        #[arg(long)]
        full: bool,
        #[arg(long)]
        wait: bool,
    },
    History {
        profile: String,
    },
    Stats {
        profile: String,
    },
    Job {
        id: String,
    },
    Cancel {
        id: String,
    },
    Destination {
        id: String,
        #[command(subcommand)]
        action: DestCmd,
    },
    Password {
        destination: String,
        #[arg(long)]
        generate: bool,
        #[arg(long)]
        stdin: bool,
    },
    ExportPassword {
        destination: String,
        path: PathBuf,
    },
    SmbPassword {
        destination: String,
        #[arg(long)]
        stdin: bool,
    },
    Approve {
        profile: String,
        #[arg(long)]
        revoke: bool,
    },
    Schedule {
        profile: String,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long)]
        disable: bool,
    },
    Schedules,
    Retention {
        profile: String,
        #[arg(long)]
        apply: bool,
        #[arg(long, requires = "apply")]
        confirm_profile: Option<String>,
    },
}
#[derive(Subcommand)]
enum ConfigCmd {
    Show,
    Import { path: PathBuf },
    Example,
}
#[derive(Subcommand)]
enum DestCmd {
    Test,
    Validate {
        profile: String,
    },
    Init {
        profile: String,
        #[arg(long)]
        confirm: bool,
    },
    Bind,
    Connect,
    Disconnect,
}
fn read_secret(stdin: bool, prompt: &str) -> Result<nyx_sync::Secret> {
    let value = if stdin {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        s.trim_end_matches(['\n', '\r']).to_owned()
    } else {
        rpassword::prompt_password(prompt)?
    };
    nyx_sync::Secret::new(value)
}
fn wait_job(id: &str) -> Result<serde_json::Value> {
    loop {
        let v: serde_json::Value =
            serde_json::from_str(&backup_service::client_call("Job", (id,))?)?;
        if v["state"] != "running" {
            return Ok(v);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}
fn start(profile: &str, op: &str, wait: bool) -> Result<serde_json::Value> {
    let id = backup_service::start(profile, op)?;
    if wait {
        wait_job(&id)
    } else {
        Ok(serde_json::json!({"job":id}))
    }
}
fn direct(
    store: &Store,
    c: &Config,
    profile: &str,
    dest: Option<&str>,
    op: &str,
) -> Result<serde_json::Value> {
    let p = c.profile(profile)?;
    if let Some(d) = dest {
        ensure!(p.destination == d, "Profile uses a different destination");
    }
    let _ = store;
    backup_service::admin_run(
        &p.id,
        op,
        if op == "retention-apply" {
            Some(&p.id)
        } else {
            None
        },
    )
}
fn main() {
    backup_core::init();
    let args = Args::parse();
    match run(&args) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            if v.get("state")
                .is_some_and(|s| s != "success" && s != "running")
            {
                std::process::exit(1)
            }
        }
        Err(e) => {
            if args.json {
                println!("{}", serde_json::json!({"version":1,"error":e.to_string()}));
            } else {
                eprintln!("{e:#}");
            }
            std::process::exit(1)
        }
    }
}
fn run(a: &Args) -> Result<serde_json::Value> {
    let store = Store::default();
    let mut c = store.load()?;
    let ok = || serde_json::json!({"version":1,"ok":true});
    let parse = |s: String| -> Result<serde_json::Value> { Ok(serde_json::from_str(&s)?) };
    match &a.command {
        Cmd::Capabilities => Ok(backup_service::capabilities()),
        Cmd::Status => parse(backup_service::status()?),
        Cmd::Jobs => parse(backup_service::client_call("Jobs", ())?),
        Cmd::Inventory => Ok(inventory()),
        Cmd::Profiles => Ok(serde_json::to_value(c.profiles)?),
        Cmd::Destinations => Ok(serde_json::to_value(c.destinations)?),
        Cmd::Config { action } => match action {
            ConfigCmd::Show => Ok(serde_json::to_value(c)?),
            ConfigCmd::Import { path } => {
                let new: Config = toml::from_str(&fs::read_to_string(path)?)?;
                store.save(&new)?;
                Ok(ok())
            }
            ConfigCmd::Example => {
                Ok(serde_json::json!({"toml":include_str!("../../../data/example.toml")}))
            }
        },
        Cmd::Preview { profile } => {
            parse(backup_service::client_call("Preview", (profile.as_str(),))?)
        }
        Cmd::Backup { profile, wait } => start(profile, "backup", *wait),
        Cmd::Verify {
            profile,
            full,
            wait,
        } => start(profile, if *full { "verify-full" } else { "verify" }, *wait),
        Cmd::History { profile } => start(profile, "history", true),
        Cmd::Stats { profile } => start(profile, "stats", true),
        Cmd::Job { id } => parse(backup_service::client_call("Job", (id.as_str(),))?),
        Cmd::Cancel { id } => {
            Ok(serde_json::json!({"message":backup_service::client_call("Cancel",(id.as_str(),))?}))
        }
        Cmd::Password {
            destination,
            generate,
            stdin,
        } => {
            c.destination(destination)?;
            let secret = if *generate {
                generated_secret()?
            } else {
                read_secret(*stdin, "Repository password: ")?
            };
            save_password(destination, &secret)?;
            Ok(ok())
        }
        Cmd::ExportPassword { destination, path } => {
            c.destination(destination)?;
            export_password(destination, path)?;
            Ok(ok())
        }
        Cmd::SmbPassword { destination, stdin } => {
            setup_smb(
                &store,
                c.destination(destination)?,
                &read_secret(*stdin, "SMB password: ")?,
            )?;
            Ok(ok())
        }
        Cmd::Approve { profile, revoke } => {
            let p = c
                .profiles
                .iter_mut()
                .find(|p| p.id == *profile)
                .context("Unknown profile")?;
            p.approved = !*revoke;
            store.save(&c)?;
            Ok(ok())
        }
        Cmd::Destination { id, action } => {
            let d = c.destination(id)?.clone();
            match action {
                DestCmd::Test => connection_test(&store, &d),
                DestCmd::Validate { profile } => direct(&store, &c, profile, Some(id), "validate"),
                DestCmd::Init { profile, confirm } => {
                    ensure!(
                        *confirm,
                        "Initialization creates a new repository. Supply --confirm explicitly; never use init to repair a failed connection."
                    );
                    direct(&store, &c, profile, Some(id), "init")
                }
                DestCmd::Bind => {
                    let path = d
                        .local_path()
                        .context("Only mounted local paths can be bound")?
                        .to_path_buf();
                    let m = mount_identity(&path)?;
                    ensure!(
                        m["target"] != "/",
                        "Select a mounted device/share, not the root filesystem"
                    );
                    let point =
                        PathBuf::from(m["target"].as_str().context("Missing mount target")?);
                    let source = m["source"]
                        .as_str()
                        .context("Missing mount source")?
                        .to_string();
                    let filesystem = m["fstype"]
                        .as_str()
                        .context("Missing filesystem type")?
                        .to_string();
                    let target = c.destinations.iter_mut().find(|x| x.id == *id).unwrap();
                    target.kind = if filesystem == "nfs" || filesystem == "nfs4" {
                        DestinationKind::Nfs {
                            path,
                            mount_point: point,
                            source,
                            filesystem,
                        }
                    } else {
                        DestinationKind::Mounted {
                            path,
                            mount_point: point,
                            source,
                            filesystem,
                            uuid: m["uuid"].as_str().unwrap_or("").into(),
                        }
                    };
                    store.save(&c)?;
                    Ok(ok())
                }
                DestCmd::Connect | DestCmd::Disconnect => {
                    let _guard = backup_engine::RepoLock::acquire(&store)?;
                    nfs_connection(&d, matches!(action, DestCmd::Connect))?;
                    Ok(ok())
                }
            }
        }
        Cmd::Schedule {
            profile,
            enable,
            disable,
        } => {
            if *enable || *disable {
                schedule(&store, c.profile(profile)?, *enable)?;
            }
            Ok(schedules())
        }
        Cmd::Schedules => Ok(schedules()),
        Cmd::Retention {
            profile,
            apply,
            confirm_profile,
        } => {
            if *apply {
                ensure!(
                    confirm_profile.as_deref() == Some(profile),
                    "Retention deletes snapshots. Pass --confirm-profile with the exact profile ID after reviewing a retention preview."
                );
            }
            direct(
                &store,
                &c,
                profile,
                None,
                if *apply {
                    "retention-apply"
                } else {
                    "retention-preview"
                },
            )
        }
    }
}
