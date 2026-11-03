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
    Github {
        #[command(subcommand)]
        action: GithubCmd,
    },
    Integrations {
        #[command(subcommand)]
        action: IntegrationCmd,
    },
    S3Credentials {
        destination: String,
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        session_token: bool,
    },
    Migration {
        #[command(subcommand)]
        action: MigrationCmd,
    },
    Setup {
        #[command(subcommand)]
        action: SetupCmd,
    },
    Restore {
        #[command(subcommand)]
        action: RestoreCmd,
    },
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
enum GithubCmd {
    Plan {
        repository: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long = "profile", required = true)]
        profiles: Vec<String>,
    },
    Inspect {
        id: String,
    },
    Push {
        id: String,
        #[arg(long)]
        confirm: String,
    },
    Fetch {
        repository: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Subcommand)]
enum IntegrationCmd {
    List,
    Grant {
        consumer: String,
        profile: String,
        #[arg(long, value_delimiter = ',', default_value = "status")]
        actions: Vec<String>,
        #[arg(long, default_value_t = 3600)]
        seconds: u64,
    },
    Revoke {
        id: String,
    },
}
#[derive(Subcommand)]
enum RestoreCmd {
    Snapshots {
        profile: String,
    },
    Browse {
        profile: String,
        snapshot: String,
        #[arg(long, default_value = "/")]
        directory: String,
    },
    Plan {
        profile: String,
        snapshot: String,
        #[arg(long = "path", required = true)]
        paths: Vec<String>,
        #[arg(long, conflicts_with = "original_dotfiles")]
        target: Option<PathBuf>,
        #[arg(long)]
        original_dotfiles: bool,
        #[arg(long,default_value="skip",value_parser=["skip","keep-both","replace"])]
        policy: String,
        #[arg(long)]
        include_secrets: bool,
        #[arg(long)]
        wait: bool,
    },
    Inspect {
        profile: String,
        plan: String,
    },
    Receipts {
        profile: String,
    },
    Diff {
        profile: String,
        plan: String,
        path: String,
    },
    Apply {
        profile: String,
        plan: String,
        #[arg(long)]
        confirm: String,
        #[arg(long)]
        wait: bool,
    },
    RollbackPreview {
        profile: String,
        plan: String,
    },
    Rollback {
        profile: String,
        plan: String,
        #[arg(long)]
        confirm: String,
        #[arg(long)]
        wait: bool,
    },
}
fn restore_command(action: &RestoreCmd) -> Result<serde_json::Value> {
    use backup_restore::{Mode, Policy, Request};
    let (profile, request, wait) = match action {
        RestoreCmd::Snapshots { profile } => (profile, Request::Snapshots, false),
        RestoreCmd::Browse {
            profile,
            snapshot,
            directory,
        } => (
            profile,
            Request::Browse {
                snapshot: snapshot.clone(),
                directory: directory.clone(),
            },
            false,
        ),
        RestoreCmd::Plan {
            profile,
            snapshot,
            paths,
            target,
            original_dotfiles,
            policy,
            include_secrets,
            wait,
        } => (
            profile,
            Request::Plan {
                snapshot: snapshot.clone(),
                paths: paths.clone(),
                target: target.clone(),
                mode: if *original_dotfiles {
                    Mode::OriginalDotfiles
                } else {
                    Mode::Separate
                },
                policy: match policy.as_str() {
                    "replace" => Policy::Replace,
                    "keep-both" => Policy::KeepBoth,
                    _ => Policy::Skip,
                },
                include_secrets: *include_secrets,
            },
            *wait,
        ),
        RestoreCmd::Inspect { profile, plan } => {
            (profile, Request::Inspect { plan: plan.clone() }, false)
        }
        RestoreCmd::Receipts { profile } => (profile, Request::Receipts, false),
        RestoreCmd::Diff {
            profile,
            plan,
            path,
        } => (
            profile,
            Request::Diff {
                plan: plan.clone(),
                path: path.clone(),
            },
            false,
        ),
        RestoreCmd::Apply {
            profile,
            plan,
            confirm,
            wait,
        } => (
            profile,
            Request::Apply {
                plan: plan.clone(),
                confirmation: confirm.clone(),
            },
            *wait,
        ),
        RestoreCmd::RollbackPreview { profile, plan } => (
            profile,
            Request::RollbackPreview { plan: plan.clone() },
            false,
        ),
        RestoreCmd::Rollback {
            profile,
            plan,
            confirm,
            wait,
        } => (
            profile,
            Request::Rollback {
                plan: plan.clone(),
                confirmation: confirm.clone(),
            },
            *wait,
        ),
    };
    if request.is_job() {
        let id = backup_service::restore_start(profile, request)?;
        if wait {
            backup_service::wait_job(&id)
        } else {
            Ok(serde_json::json!({"job":id}))
        }
    } else {
        backup_service::restore_read(profile, request)
    }
}
#[derive(Subcommand)]
enum MigrationCmd {
    Capture {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Source {
        profile: String,
        snapshot: String,
    },
    Prepare {
        profile: String,
        snapshot: String,
        #[arg(long = "path")]
        paths: Vec<String>,
        #[arg(long)]
        source_manifest: Option<PathBuf>,
        #[arg(long,default_value="skip",value_parser=["skip","keep-both","replace"])]
        policy: String,
        #[arg(long)]
        include_secrets: bool,
        #[arg(long)]
        no_rewrite_references: bool,
        #[arg(long)]
        wait: bool,
    },
    Inspect {
        profile: String,
        bundle: String,
    },
    Bundles {
        profile: String,
    },
    Export {
        profile: String,
        bundle: String,
        path: PathBuf,
    },
}
#[derive(Subcommand)]
enum SetupCmd {
    Inspect {
        bundle: PathBuf,
    },
    Prepare {
        profile: String,
        snapshot: String,
        #[arg(long = "path")]
        paths: Vec<String>,
        #[arg(long)]
        source_manifest: Option<PathBuf>,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long,default_value="skip",value_parser=["skip","keep-both","replace"])]
        policy: String,
        #[arg(long)]
        include_secrets: bool,
    },
    Packages {
        bundle: PathBuf,
        #[arg(long)]
        apply: bool,
        #[arg(long, requires = "apply")]
        confirm: Option<String>,
    },
    Restore {
        bundle: PathBuf,
        #[arg(long)]
        confirm: String,
        #[arg(long)]
        wait: bool,
    },
}
fn load_inventory(path: &PathBuf) -> Result<backup_core::migration::Inventory> {
    ensure!(
        fs::metadata(path)?.len() <= 8 * 1024 * 1024 && fs::metadata(path)?.is_file(),
        "Source manifest must be a bounded ordinary JSON file"
    );
    let value: backup_core::migration::Inventory = serde_json::from_slice(&fs::read(path)?)?;
    value.validate()?;
    Ok(value)
}
fn migration_request(
    snapshot: &str,
    paths: &[String],
    source: &Option<PathBuf>,
    policy: &str,
    secrets: bool,
    rewrite: bool,
) -> Result<backup_migrate::Request> {
    Ok(backup_migrate::Request::Prepare {
        snapshot: snapshot.into(),
        paths: paths.to_vec(),
        source: source.as_ref().map(load_inventory).transpose()?,
        policy: match policy {
            "replace" => backup_restore::Policy::Replace,
            "keep-both" => backup_restore::Policy::KeepBoth,
            _ => backup_restore::Policy::Skip,
        },
        include_secrets: secrets,
        rewrite_references: rewrite,
    })
}
fn migration_command(action: &MigrationCmd) -> Result<serde_json::Value> {
    use backup_migrate::Request;
    match action {
        MigrationCmd::Capture { output } => {
            let inventory = backup_system::migration_inventory::capture();
            inventory.validate()?;
            if let Some(path) = output {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(path)?;
                f.write_all(&serde_json::to_vec_pretty(&inventory)?)?;
                f.sync_all()?;
                Ok(serde_json::json!({"version":1,"exported":path}))
            } else {
                Ok(serde_json::to_value(inventory)?)
            }
        }
        MigrationCmd::Source { profile, snapshot } => backup_service::migration_read(
            profile,
            Request::Source {
                snapshot: snapshot.clone(),
            },
        ),
        MigrationCmd::Prepare {
            profile,
            snapshot,
            paths,
            source_manifest,
            policy,
            include_secrets,
            no_rewrite_references,
            wait,
        } => {
            let id = backup_service::migration_start(
                profile,
                migration_request(
                    snapshot,
                    paths,
                    source_manifest,
                    policy,
                    *include_secrets,
                    !*no_rewrite_references,
                )?,
            )?;
            if *wait {
                backup_service::wait_job(&id)
            } else {
                Ok(serde_json::json!({"job":id}))
            }
        }
        MigrationCmd::Inspect { profile, bundle } => backup_service::migration_read(
            profile,
            Request::Inspect {
                bundle: bundle.clone(),
            },
        ),
        MigrationCmd::Bundles { profile } => {
            backup_service::migration_read(profile, Request::Bundles)
        }
        MigrationCmd::Export {
            profile,
            bundle,
            path,
        } => backup_service::migration_read(
            profile,
            Request::Export {
                bundle: bundle.clone(),
                path: path.clone(),
            },
        ),
    }
}
fn setup_command(action: &SetupCmd) -> Result<serde_json::Value> {
    match action {
        SetupCmd::Inspect { bundle } => {
            backup_migrate::setup_preview(&backup_migrate::import_bundle(bundle)?)
        }
        SetupCmd::Packages {
            bundle,
            apply,
            confirm,
        } => {
            let bundle = backup_migrate::import_bundle(bundle)?;
            if *apply {
                backup_migrate::setup_install(
                    &bundle,
                    confirm
                        .as_deref()
                        .context("Supply --confirm with the exact Setup bundle ID")?,
                )?;
                Ok(
                    serde_json::json!({"version":1,"bundle":bundle.id,"ok":true,"message":"Package handoff finished; configuration restore remains separate"}),
                )
            } else {
                backup_migrate::setup_preview(&bundle)
            }
        }
        SetupCmd::Restore {
            bundle,
            confirm,
            wait,
        } => {
            let bundle = backup_migrate::import_bundle(bundle)?;
            ensure!(*confirm == bundle.id, "Confirm the exact Setup bundle ID");
            let _ = backup_migrate::setup_preview(&bundle)?;
            let plan = bundle
                .restore_plan
                .context("This bundle has no staged configuration plan")?;
            let id = backup_service::restore_start(
                &bundle.profile,
                backup_restore::Request::Apply {
                    plan: plan.clone(),
                    confirmation: plan,
                },
            )?;
            if *wait {
                backup_service::wait_job(&id)
            } else {
                Ok(serde_json::json!({"job":id}))
            }
        }
        SetupCmd::Prepare {
            profile,
            snapshot,
            paths,
            source_manifest,
            bundle,
            policy,
            include_secrets,
        } => {
            let id = backup_service::migration_start(
                profile,
                migration_request(
                    snapshot,
                    paths,
                    source_manifest,
                    policy,
                    *include_secrets,
                    true,
                )?,
            )?;
            let job = backup_service::wait_job(&id)?;
            if job["state"] != "success" {
                return Ok(job);
            }
            let id = job["result"]["bundle"]
                .as_str()
                .context("Missing Setup bundle ID")?;
            backup_service::migration_read(
                profile,
                backup_migrate::Request::Export {
                    bundle: id.into(),
                    path: bundle.clone(),
                },
            )
        }
    }
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
            if v.get("operation").is_some()
                && v.get("state")
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
        Cmd::Migration { action } => migration_command(action),
        Cmd::Setup { action } => setup_command(action),
        Cmd::Restore { action } => restore_command(action),
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
        Cmd::Cancel { id } => Ok(serde_json::json!({"message":backup_service::cancel(id)?})),
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
        Cmd::Github { action } => {
            use backup_service::github_sync::*;
            match action {
                GithubCmd::Plan {
                    repository,
                    branch,
                    profiles,
                } => prepare(&store, repository, branch, profiles),
                GithubCmd::Inspect { id } => inspect(&store, id),
                GithubCmd::Push { id, confirm } => push(&store, id, confirm),
                GithubCmd::Fetch {
                    repository,
                    branch,
                    output,
                } => fetch(repository, branch, output),
            }
        }
        Cmd::Integrations { action } => {
            use backup_service::integration::*;
            match action {
                IntegrationCmd::List => Ok(serde_json::to_value(list(&store)?)?),
                IntegrationCmd::Grant {
                    consumer,
                    profile,
                    actions,
                    seconds,
                } => Ok(serde_json::to_value(issue(
                    &store, consumer, profile, actions, *seconds,
                )?)?),
                IntegrationCmd::Revoke { id } => {
                    revoke(&store, id)?;
                    Ok(ok())
                }
            }
        }
        Cmd::S3Credentials {
            destination,
            stdin,
            session_token,
        } => {
            let access = read_secret(*stdin, "S3 access key ID: ")?;
            let key = read_secret(*stdin, "S3 secret access key: ")?;
            let token = if *session_token {
                Some(read_secret(*stdin, "S3 session token: ")?)
            } else {
                None
            };
            setup_s3(
                &store,
                c.destination(destination)?,
                &access,
                &key,
                token.as_ref(),
            )?;
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
