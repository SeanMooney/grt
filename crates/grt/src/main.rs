// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use tracing::debug;

use grt::app::App;
use grt::comments;
use grt::config::CliOverrides;
use grt::export::{self, ExportArgs};
use grt::hook;
use grt::push::{self, PushOptions};
use grt::review::ReviewArgs;
use grt::subprocess;

/// grt — CLI/TUI tool for Git and Gerrit workflows
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Run as if started in <PATH>
    #[arg(short = 'C', long = "directory", global = true)]
    directory: Option<PathBuf>,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    /// Allow sending credentials over plain HTTP (no TLS)
    #[arg(long, global = true)]
    insecure: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Push changes to Gerrit for review (git-review compatible)
    Review(ReviewArgs),

    /// Push changes to Gerrit for review
    Push(PushArgs),

    /// Retrieve review comments from Gerrit
    Comments(CommentsArgs),

    /// Set up current repo for Gerrit (hook, remote, connectivity)
    Setup(SetupArgs),

    /// Export grt functionality (e.g., create git-review symlink)
    Export(ExportArgs),

    /// Show grt and Gerrit server versions
    Version,
}

/// git-review compatible CLI — used when invoked as `git-review` via argv[0].
#[derive(Parser, Debug)]
#[command(
    name = "git-review",
    version,
    about = "Push changes to Gerrit for review"
)]
struct GitReviewCli {
    /// Increase verbosity (-v, -vv, -vvv)
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Colorize output
    #[arg(long, value_name = "WHEN")]
    color: Option<String>,

    /// Disable colored output
    #[arg(long)]
    no_color: bool,

    #[command(flatten)]
    review: ReviewArgs,
}

#[derive(Parser, Debug)]
struct PushArgs {
    /// Target branch (defaults to config or "main")
    branch: Option<String>,

    /// Remote to push to
    #[arg(long)]
    remote: Option<String>,

    /// Topic for the change
    #[arg(long)]
    topic: Option<String>,

    /// Mark as work-in-progress
    #[arg(long)]
    wip: bool,

    /// Mark as ready for review
    #[arg(long)]
    ready: bool,

    /// Mark as private
    #[arg(long)]
    private: bool,

    /// Remove private flag
    #[arg(long)]
    remove_private: bool,

    /// Add reviewers (comma-separated or repeated)
    #[arg(short = 'r', long, value_delimiter = ',')]
    reviewers: Vec<String>,

    /// Add CC recipients (comma-separated or repeated)
    #[arg(long, value_delimiter = ',')]
    cc: Vec<String>,

    /// Add hashtags (comma-separated or repeated)
    #[arg(long, value_delimiter = ',')]
    hashtags: Vec<String>,

    /// Review message
    #[arg(short, long)]
    message: Option<String>,

    /// Notification setting (NONE, OWNER, etc.)
    #[arg(long)]
    notify: Option<String>,

    /// Disable automatic rebase
    #[arg(long)]
    no_rebase: bool,

    /// Show what would be pushed without pushing
    #[arg(long)]
    dry_run: bool,

    /// Skip confirmation prompt
    #[arg(short, long)]
    yes: bool,

    /// Generate a new Change-Id (amend HEAD)
    #[arg(long)]
    new_changeid: bool,
}

#[derive(Parser, Debug)]
struct CommentsArgs {
    /// Change number or Change-Id (auto-detected from HEAD if omitted)
    change: Option<String>,

    /// Patchset revision to show comments for
    #[arg(long)]
    revision: Option<String>,

    /// Show only unresolved comments
    #[arg(long)]
    unresolved: bool,

    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,

    /// Show comments from all revisions
    #[arg(long)]
    all_revisions: bool,

    /// Include robot/automated comments
    #[arg(long)]
    include_robot_comments: bool,
}

#[derive(Parser, Debug)]
struct SetupArgs {
    /// Remote name to configure
    #[arg(long)]
    remote: Option<String>,

    /// Force reinstall of commit-msg hook even if it exists
    #[arg(long)]
    force_hook: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

/// CLI personality based on argv[0].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Personality {
    /// Normal `grt` invocation with subcommands.
    Grt,
    /// Busybox-style `git-review` invocation with flat flags.
    GitReview,
}

/// Detect CLI personality from argv[0].
fn detect_personality(argv0: &str) -> Personality {
    let basename = Path::new(argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    if basename == "git-review" {
        Personality::GitReview
    } else {
        Personality::Grt
    }
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    let filter = format!("grt={level}");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&filter)),
        )
        .with_target(false)
        .without_time()
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let argv0 = std::env::args().next().unwrap_or_default();
    let personality = detect_personality(&argv0);

    match personality {
        Personality::GitReview => {
            let cli = GitReviewCli::parse();
            init_tracing(cli.verbose);

            let work_dir = std::env::current_dir().expect("cannot determine current directory");
            cmd_review(&work_dir, cli.review, false).await
        }
        Personality::Grt => {
            let cli = Cli::parse();
            init_tracing(cli.verbose);

            let work_dir = cli.directory.unwrap_or_else(|| {
                std::env::current_dir().expect("cannot determine current directory")
            });

            let insecure = cli.insecure;
            match cli.command {
                Commands::Review(args) => cmd_review(&work_dir, args, insecure).await,
                Commands::Push(args) => cmd_push(&work_dir, args, insecure).await,
                Commands::Comments(args) => cmd_comments(&work_dir, args, insecure).await,
                Commands::Setup(args) => cmd_setup(&work_dir, args, insecure).await,
                Commands::Export(args) => export::cmd_export(&args),
                Commands::Version => cmd_version(&work_dir).await,
            }
        }
    }
}

/// Dispatch `grt review` / `git-review` based on which mode flag is set.
async fn cmd_review(work_dir: &Path, args: ReviewArgs, insecure: bool) -> Result<()> {
    // Setup mode
    if args.setup {
        return cmd_setup(
            work_dir,
            SetupArgs {
                remote: args.remote,
                force_hook: false,
            },
            insecure,
        )
        .await;
    }

    // Modes not yet implemented (Batch 2+)
    if args.download.is_some() {
        anyhow::bail!("download mode (-d) is not yet implemented");
    }
    if args.cherrypick.is_some() {
        anyhow::bail!("cherrypick mode (-x) is not yet implemented");
    }
    if args.cherrypickindicate.is_some() {
        anyhow::bail!("cherrypickindicate mode (-X) is not yet implemented");
    }
    if args.cherrypickonly.is_some() {
        anyhow::bail!("cherrypickonly mode (-N) is not yet implemented");
    }
    if args.compare.is_some() {
        anyhow::bail!("compare mode (-m) is not yet implemented");
    }
    if args.list > 0 {
        anyhow::bail!("list mode (-l) is not yet implemented");
    }

    // Default mode: push
    cmd_push(
        work_dir,
        PushArgs {
            branch: args.branch,
            remote: args.remote,
            topic: if args.no_topic { None } else { args.topic },
            wip: args.wip,
            ready: args.ready,
            private: args.private,
            remove_private: args.remove_private,
            reviewers: args.reviewers,
            cc: args.cc,
            hashtags: args.hashtags,
            message: args.message,
            notify: args.notify,
            no_rebase: args.no_rebase,
            dry_run: args.dry_run,
            yes: args.yes,
            new_changeid: args.new_changeid,
        },
        insecure,
    )
    .await
}

async fn cmd_push(work_dir: &Path, args: PushArgs, insecure: bool) -> Result<()> {
    let cli_overrides = CliOverrides {
        remote: args.remote.clone(),
        branch: args.branch.clone(),
        insecure,
        ..Default::default()
    };
    let app = App::new(work_dir, &cli_overrides)?;
    let root = app.git.root()?;

    // Ensure commit-msg hook is installed
    let hooks_dir = app.git.hooks_dir()?;
    hook::ensure_hook_installed(&hooks_dir)?;
    debug!("commit-msg hook verified at {:?}", hooks_dir);

    let branch = args.branch.unwrap_or_else(|| app.config.branch.clone());
    let remote = args.remote.unwrap_or_else(|| app.config.remote.clone());

    // Validate Change-Id in HEAD
    let commit_msg = app.git.head_commit_message()?;
    push::validate_change_id(&commit_msg)?;

    // Count unpushed commits
    let count = subprocess::count_unpushed_commits(&remote, &branch, &root)?;
    if count == 0 {
        eprintln!("No unpushed commits found.");
        return Ok(());
    }

    if count > 1 && !args.yes {
        eprintln!("About to push {count} commit(s) to {remote}/{branch}. Continue? [y/N] ");
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .context("reading user input")?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Push cancelled.");
            return Ok(());
        }
    }

    let opts = PushOptions {
        branch: branch.clone(),
        topic: args.topic,
        wip: args.wip,
        ready: args.ready,
        private: args.private,
        remove_private: args.remove_private,
        reviewers: args.reviewers,
        cc: args.cc,
        hashtags: args.hashtags,
        message: args.message,
        notify: args.notify,
        no_rebase: args.no_rebase,
    };

    let refspec = push::build_refspec(&opts)?;

    if args.dry_run {
        println!("git push {remote} {refspec}");
        return Ok(());
    }

    eprintln!("Pushing {count} commit(s) to {remote}/{branch}...");
    subprocess::git_exec(&["push", &remote, &refspec], &root)?;
    eprintln!("Push successful.");
    Ok(())
}

async fn cmd_comments(work_dir: &Path, args: CommentsArgs, insecure: bool) -> Result<()> {
    let cli_overrides = CliOverrides {
        insecure,
        ..Default::default()
    };
    let mut app = App::new(work_dir, &cli_overrides)?;

    // Authenticate for API access and verify credentials
    app.authenticate_and_verify().await?;

    // Determine change identifier
    let change_id = match args.change {
        Some(id) => id,
        None => {
            let msg = app.git.head_commit_message()?;
            push::extract_change_id(&msg)
                .context("no Change-Id found in HEAD commit. Specify a change number explicitly")?
        }
    };

    debug!("fetching comments for change: {}", change_id);

    // Fetch change detail and comments
    let change = app.gerrit.get_change_detail(&change_id).await?;
    let change_comments = if args.all_revisions {
        app.gerrit.get_change_comments(&change_id).await?
    } else if let Some(ref rev) = args.revision {
        app.gerrit.get_revision_comments(&change_id, rev).await?
    } else if let Some(ref current_rev) = change.current_revision {
        app.gerrit
            .get_revision_comments(&change_id, current_rev)
            .await?
    } else {
        app.gerrit.get_change_comments(&change_id).await?
    };

    let mut all_comments = change_comments;

    // Optionally include robot comments
    if args.include_robot_comments {
        if let Ok(robot) = app.gerrit.get_robot_comments(&change_id).await {
            for (file, comments) in robot {
                all_comments.entry(file).or_default().extend(comments);
            }
        }
    }

    let mut threads = comments::build_threads(&all_comments);

    // Filter to unresolved only if requested
    if args.unresolved {
        threads.retain(|t| !t.resolved);
    }

    let messages = change.messages.as_deref().unwrap_or(&[]);
    let gerrit_url = app.config.gerrit_base_url()?.to_string();

    match args.format {
        OutputFormat::Text => {
            let text = comments::format_text(&change, messages, &threads, &gerrit_url);
            print!("{text}");
        }
        OutputFormat::Json => {
            let json = comments::format_json(&change, messages, &threads, &gerrit_url);
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }

    Ok(())
}

async fn cmd_setup(work_dir: &Path, args: SetupArgs, insecure: bool) -> Result<()> {
    let cli_overrides = CliOverrides {
        remote: args.remote.clone(),
        insecure,
        ..Default::default()
    };
    let mut app = App::new(work_dir, &cli_overrides)?;
    let root = app.git.root()?;

    eprintln!("Setting up grt for Gerrit...");

    // 1. Install commit-msg hook
    let hooks_dir = app.git.hooks_dir()?;
    let hook_path = hooks_dir.join("commit-msg");
    if args.force_hook && hook_path.exists() {
        std::fs::remove_file(&hook_path).context("removing existing commit-msg hook")?;
    }
    hook::ensure_hook_installed(&hooks_dir)?;
    eprintln!("  commit-msg hook: installed at {}", hook_path.display());

    // 2. Verify remote exists
    let remote = args.remote.unwrap_or_else(|| app.config.remote.clone());
    let remote_check = subprocess::git_output(&["remote", "get-url", &remote], &root);
    match remote_check {
        Ok(url) => eprintln!("  remote '{remote}': {url}"),
        Err(_) => eprintln!("  remote '{remote}': NOT FOUND (you may need to add it)"),
    }

    // 3. Test connectivity and auth
    eprintln!("  Gerrit host: {}", app.config.host);
    match app.gerrit.get_version().await {
        Ok(version) => {
            eprintln!("  connectivity: OK (Gerrit {version})");
        }
        Err(e) => {
            eprintln!("  connectivity: FAILED ({e})");
            eprintln!("  Trying with authentication...");
            if app.authenticate().is_ok() {
                match app.gerrit.get_version().await {
                    Ok(version) => {
                        eprintln!("  connectivity: OK (Gerrit {version}, authenticated)")
                    }
                    Err(e) => eprintln!("  connectivity: FAILED with auth ({e})"),
                }
            } else {
                eprintln!("  authentication: FAILED (check git credentials)");
            }
        }
    }

    // 4. Verify auth
    match app.gerrit.get_self_account().await {
        Ok(account) => {
            let name = account.name.as_deref().unwrap_or("unknown");
            let email = account.email.as_deref().unwrap_or("unknown");
            eprintln!("  authenticated as: {name} <{email}>");
        }
        Err(_) => {
            if app.authenticate().is_ok() {
                match app.gerrit.get_self_account().await {
                    Ok(account) => {
                        let name = account.name.as_deref().unwrap_or("unknown");
                        let email = account.email.as_deref().unwrap_or("unknown");
                        eprintln!("  authenticated as: {name} <{email}>");
                    }
                    Err(e) => eprintln!("  auth check: FAILED ({e})"),
                }
            }
        }
    }

    eprintln!("\nSetup complete.");
    Ok(())
}

async fn cmd_version(work_dir: &Path) -> Result<()> {
    println!("grt {}", env!("CARGO_PKG_VERSION"));

    // Try to get Gerrit version
    let cli_overrides = CliOverrides::default();
    match App::new(work_dir, &cli_overrides) {
        Ok(app) => match app.gerrit.get_version().await {
            Ok(version) => println!("Gerrit {version}"),
            Err(_) => println!("Gerrit version: unavailable"),
        },
        Err(_) => println!("Gerrit version: unavailable (not in a configured repository)"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_cli() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn verify_git_review_cli() {
        use clap::CommandFactory;
        GitReviewCli::command().debug_assert();
    }

    // === Existing push/comments/setup/version tests ===

    #[test]
    fn parse_push_defaults() {
        let cli = Cli::parse_from(["grt", "push"]);
        assert!(matches!(cli.command, Commands::Push(_)));
    }

    #[test]
    fn parse_push_all_flags() {
        let cli = Cli::parse_from([
            "grt", "push", "--topic", "foo", "--wip", "-r", "alice", "main",
        ]);
        if let Commands::Push(args) = cli.command {
            assert_eq!(args.topic.as_deref(), Some("foo"));
            assert!(args.wip);
            assert_eq!(args.reviewers, vec!["alice"]);
            assert_eq!(args.branch.as_deref(), Some("main"));
        } else {
            panic!("expected Push command");
        }
    }

    #[test]
    fn parse_comments_defaults() {
        let cli = Cli::parse_from(["grt", "comments"]);
        assert!(matches!(cli.command, Commands::Comments(_)));
    }

    #[test]
    fn parse_comments_with_change() {
        let cli = Cli::parse_from([
            "grt",
            "comments",
            "12345",
            "--unresolved",
            "--format",
            "json",
        ]);
        if let Commands::Comments(args) = cli.command {
            assert_eq!(args.change.as_deref(), Some("12345"));
            assert!(args.unresolved);
            assert!(matches!(args.format, OutputFormat::Json));
        } else {
            panic!("expected Comments command");
        }
    }

    #[test]
    fn parse_setup_defaults() {
        let cli = Cli::parse_from(["grt", "setup"]);
        assert!(matches!(cli.command, Commands::Setup(_)));
    }

    #[test]
    fn parse_version() {
        let cli = Cli::parse_from(["grt", "version"]);
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn parse_global_verbose() {
        let cli = Cli::parse_from(["grt", "-vvv", "version"]);
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn parse_global_directory() {
        let cli = Cli::parse_from(["grt", "-C", "/tmp", "version"]);
        assert_eq!(cli.directory, Some(PathBuf::from("/tmp")));
    }

    // === New: review subcommand tests ===

    #[test]
    fn parse_review_subcommand() {
        let cli = Cli::parse_from(["grt", "review", "main"]);
        if let Commands::Review(args) = cli.command {
            assert_eq!(args.branch.as_deref(), Some("main"));
        } else {
            panic!("expected Review command");
        }
    }

    #[test]
    fn parse_review_with_download() {
        let cli = Cli::parse_from(["grt", "review", "-d", "12345"]);
        if let Commands::Review(args) = cli.command {
            assert_eq!(args.download.as_deref(), Some("12345"));
        } else {
            panic!("expected Review command");
        }
    }

    #[test]
    fn parse_review_with_setup() {
        let cli = Cli::parse_from(["grt", "review", "-s"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.setup);
        } else {
            panic!("expected Review command");
        }
    }

    #[test]
    fn parse_review_push_mode_flags() {
        let cli = Cli::parse_from([
            "grt", "review", "-w", "-t", "my-topic", "-r", "origin", "main",
        ]);
        if let Commands::Review(args) = cli.command {
            assert!(args.wip);
            assert_eq!(args.topic.as_deref(), Some("my-topic"));
            assert_eq!(args.remote.as_deref(), Some("origin"));
            assert_eq!(args.branch.as_deref(), Some("main"));
        } else {
            panic!("expected Review command");
        }
    }

    #[test]
    fn parse_review_no_args() {
        let cli = Cli::parse_from(["grt", "review"]);
        assert!(matches!(cli.command, Commands::Review(_)));
    }

    // === New: export subcommand tests ===

    #[test]
    fn parse_export_git_review() {
        let cli = Cli::parse_from(["grt", "export", "git-review"]);
        if let Commands::Export(args) = cli.command {
            assert!(matches!(
                args.target,
                export::ExportTarget::GitReview { clean: false }
            ));
        } else {
            panic!("expected Export command");
        }
    }

    #[test]
    fn parse_export_git_review_clean() {
        let cli = Cli::parse_from(["grt", "export", "git-review", "--clean"]);
        if let Commands::Export(args) = cli.command {
            assert!(matches!(
                args.target,
                export::ExportTarget::GitReview { clean: true }
            ));
        } else {
            panic!("expected Export command");
        }
    }

    // === New: argv[0] personality detection tests ===

    #[test]
    fn detect_personality_grt_bare() {
        assert_eq!(detect_personality("grt"), Personality::Grt);
    }

    #[test]
    fn detect_personality_grt_absolute() {
        assert_eq!(detect_personality("/usr/bin/grt"), Personality::Grt);
    }

    #[test]
    fn detect_personality_git_review_bare() {
        assert_eq!(detect_personality("git-review"), Personality::GitReview);
    }

    #[test]
    fn detect_personality_git_review_absolute() {
        assert_eq!(
            detect_personality("/usr/local/bin/git-review"),
            Personality::GitReview
        );
    }

    #[test]
    fn detect_personality_git_review_home_local() {
        assert_eq!(
            detect_personality("/home/user/.local/bin/git-review"),
            Personality::GitReview
        );
    }

    #[test]
    fn detect_personality_unknown_defaults_to_grt() {
        assert_eq!(detect_personality("something-else"), Personality::Grt);
    }

    // === New: git-review mode parsing tests ===

    #[test]
    fn git_review_parse_branch() {
        let cli = GitReviewCli::parse_from(["git-review", "main"]);
        assert_eq!(cli.review.branch.as_deref(), Some("main"));
    }

    #[test]
    fn git_review_parse_download() {
        let cli = GitReviewCli::parse_from(["git-review", "-d", "12345"]);
        assert_eq!(cli.review.download.as_deref(), Some("12345"));
    }

    #[test]
    fn git_review_parse_verbose() {
        let cli = GitReviewCli::parse_from(["git-review", "-v", "-d", "12345"]);
        assert_eq!(cli.verbose, 1);
        assert_eq!(cli.review.download.as_deref(), Some("12345"));
    }

    #[test]
    fn git_review_parse_no_color() {
        let cli = GitReviewCli::parse_from(["git-review", "--no-color", "main"]);
        assert!(cli.no_color);
        assert_eq!(cli.review.branch.as_deref(), Some("main"));
    }

    #[test]
    fn git_review_parse_list() {
        let cli = GitReviewCli::parse_from(["git-review", "-l"]);
        assert_eq!(cli.review.list, 1);
    }

    #[test]
    fn git_review_parse_setup() {
        let cli = GitReviewCli::parse_from(["git-review", "-s"]);
        assert!(cli.review.setup);
    }
}
