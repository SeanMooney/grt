// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2026 grt contributors

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use tracing::debug;

use grt::app::App;
use grt::comments;
use grt::config::CliOverrides;
use grt::export::{self, ExportArgs};
use grt::gerrit::GerritError;
use grt::hook;
use grt::push::{self, ChangeIdStatus, PushOptions, PushResult};
use grt::rebase;
use grt::review::{self, OutputFormat, ReviewArgs};
use grt::review_query;
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

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
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

    /// Show license information
    #[arg(long)]
    license: bool,

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

    /// Force rebase before pushing
    #[arg(long)]
    force_rebase: bool,

    /// Keep rebase state on failure (don't abort)
    #[arg(long)]
    keep_rebase: bool,

    /// Show what would be pushed without pushing
    #[arg(long)]
    dry_run: bool,

    /// Skip confirmation prompt
    #[arg(short, long)]
    yes: bool,

    /// Generate a new Change-Id (amend HEAD)
    #[arg(long)]
    new_changeid: bool,

    /// Disable thin pack for push
    #[arg(long)]
    no_thin: bool,

    /// Color for git push (e.g. always, never, auto). Set by caller, not CLI.
    #[arg(skip)]
    pub color_remote: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
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

    /// Filter threads by commenter (email, name, or username substring match)
    #[arg(long)]
    comment_by: Option<String>,

    /// Only show threads with 2+ comments (i.e., that received replies)
    #[arg(long)]
    has_replies: bool,

    /// Filter threads by label vote (e.g., Code-Review=-1)
    #[arg(long)]
    label: Option<String>,

    /// Filter comments posted after this date (YYYY-MM-DD)
    #[arg(long)]
    after: Option<String>,

    /// Filter comments posted before this date (YYYY-MM-DD)
    #[arg(long)]
    before: Option<String>,

    /// Project to search (enables cross-change search mode when no change is given)
    #[arg(long)]
    project: Option<String>,

    /// Show only comments no older than this age (e.g., 30d, 2w, 1m, 1y).
    /// In cross-change search mode this is also passed to Gerrit as -age:<value>.
    #[arg(long)]
    age: Option<String>,

    /// Show only comments at least this old (e.g., 30d, 2w).
    /// Useful for finding stale unresolved threads.
    #[arg(long)]
    min_age: Option<String>,
}

#[derive(Parser, Debug)]
struct SetupArgs {
    /// Remote name to configure
    #[arg(long)]
    remote: Option<String>,

    /// Force reinstall of commit-msg hook even if it exists
    #[arg(long)]
    force_hook: bool,

    /// Download hook from remote Gerrit server instead of using vendored copy
    #[arg(long)]
    remote_hook: bool,

    /// Use SSH transport for the Gerrit remote (default)
    #[arg(long, conflicts_with = "http")]
    ssh: bool,

    /// Use HTTPS transport for the Gerrit remote
    #[arg(long, conflicts_with = "ssh")]
    http: bool,
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

/// Map an error to an exit code for git-review compatibility.
fn exit_code_for_error(err: &anyhow::Error) -> i32 {
    // Check for GerritError in the error chain
    if let Some(gerrit_err) = err.downcast_ref::<GerritError>() {
        return match gerrit_err {
            GerritError::AuthFailed { .. } => 1,
            GerritError::NotFound => 1,
            GerritError::ServerError { .. } => 1,
            GerritError::Network(_) => 40,
        };
    }

    // Check for common error patterns in the message
    let msg = format!("{err:#}");
    if msg.contains("git config") || msg.contains("no Gerrit host configured") {
        return 128;
    }
    if msg.contains("argument") || msg.contains("CHANGE,PS") || msg.contains("malformed") {
        return 3;
    }
    if msg.contains("hook") {
        return 2;
    }

    1 // generic error
}

fn cmd_completions(shell: clap_complete::Shell) {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "grt", &mut std::io::stdout());
}

/// Resolve color.remote value from CLI flags.
fn resolve_color_remote(no_color: bool, color: Option<&str>) -> String {
    if no_color {
        return "never".to_string();
    }
    match color {
        Some("always") | Some("never") | Some("auto") => color.unwrap().to_string(),
        _ => "always".to_string(),
    }
}

/// Prompt the user for their Gerrit username on stderr, read from stdin.
/// Returns an error if stdin is not a tty or input is empty.
fn prompt_for_username() -> Result<String> {
    use std::io::IsTerminal as _;

    if !std::io::stdin().is_terminal() {
        anyhow::bail!("stdin is not a tty; set gitreview.username in git config");
    }
    eprint!("Enter your Gerrit username: ");
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading username from stdin")?;
    let username = line.trim().to_string();
    if username.is_empty() {
        anyhow::bail!("Gerrit username cannot be empty");
    }
    Ok(username)
}

/// Check if the configured remote exists, and create it if possible.
///
/// - If remote exists with a tracking branch: no-op
/// - If remote exists but no tracking branch: run `git remote update`
/// - If remote doesn't exist and config has enough info: create it
fn check_and_create_remote(app: &mut App) -> Result<()> {
    let remote = app.config.remote.clone();
    let root = app.git.root()?;

    match subprocess::check_remote_exists(&remote, &root)? {
        Some(_url) => {
            // Remote exists — check if tracking branch exists
            let tracking_ref = format!("refs/remotes/{}/{}", remote, app.config.branch);
            let has_tracking =
                subprocess::git_output(&["show-ref", "--verify", "--quiet", &tracking_ref], &root)
                    .is_ok();

            if !has_tracking {
                tracing::info!("Remote '{remote}' exists but has no tracking branch, updating...");
                subprocess::git_remote_update(&remote, &root)?;
            }
        }
        None => {
            // Remote doesn't exist — try to create from config
            if app.config.host.is_empty() {
                return Ok(()); // Not enough config to auto-create
            }

            // For SSH remotes, prompt for username if not configured
            if app.config.scheme.starts_with("ssh") && app.config.username.is_none() {
                let username = prompt_for_username()?;
                app.config.username = Some(username.clone());
                // Persist to git config so it's not asked again
                subprocess::git_exec(&["config", "gitreview.username", &username], &root)?;
            }

            let url = app.config.make_remote_url();
            tracing::info!("Creating remote '{remote}' with URL {url}...");
            subprocess::git_remote_add(&remote, &url, &root)?;

            // Set push URL if usepushurl is configured
            if app.config.usepushurl {
                let push_url = app.config.make_remote_url();
                subprocess::git_remote_set_push_url(&remote, &push_url, &root)?;
            }
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let argv0 = std::env::args().next().unwrap_or_default();
    let personality = detect_personality(&argv0);

    let result = match personality {
        Personality::GitReview => {
            let cli = GitReviewCli::parse();
            init_tracing(cli.verbose);

            if cli.license {
                println!("Licensed under Apache-2.0 OR MIT");
                return;
            }

            let work_dir = std::env::current_dir().expect("cannot determine current directory");
            let color = resolve_color_remote(cli.no_color, cli.color.as_deref());
            cmd_review(&work_dir, cli.review, false, Some(color)).await
        }
        Personality::Grt => {
            let cli = Cli::parse();
            init_tracing(cli.verbose);

            let work_dir = cli.directory.unwrap_or_else(|| {
                std::env::current_dir().expect("cannot determine current directory")
            });

            let insecure = cli.insecure;
            let color = resolve_color_remote(cli.no_color, None);
            match cli.command {
                Commands::Review(args) => cmd_review(&work_dir, args, insecure, Some(color)).await,
                Commands::Push(args) => {
                    let mut push_args = args;
                    push_args.color_remote = Some(color);
                    cmd_push(&work_dir, push_args, insecure).await
                }
                Commands::Comments(args) => cmd_comments(&work_dir, args, insecure).await,
                Commands::Setup(args) => cmd_setup(&work_dir, args, insecure).await,
                Commands::Export(args) => export::cmd_export(&args),
                Commands::Version => cmd_version(&work_dir).await,
                Commands::Completions { shell } => {
                    cmd_completions(shell);
                    Ok(())
                }
            }
        }
    };

    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(exit_code_for_error(&err));
    }
}

/// Dispatch `grt review` / `git-review` based on which mode flag is set.
async fn cmd_review(
    work_dir: &Path,
    args: ReviewArgs,
    insecure: bool,
    color_remote: Option<String>,
) -> Result<()> {
    // Setup mode: run setup, but continue if --finish is also set
    if args.setup {
        cmd_setup(
            work_dir,
            SetupArgs {
                remote: args.remote.clone(),
                force_hook: false,
                remote_hook: args.remote_hook,
                ssh: false,
                http: false,
            },
            insecure,
        )
        .await?;
        if !args.finish {
            return Ok(());
        }
        // Fall through to finish logic below
    }

    // Warn about flags that are parsed but not yet fully implemented
    review::warn_unused_flags(&args);

    // Create a single App instance for all mode dispatches
    let cli_overrides = CliOverrides {
        remote: args.remote.clone(),
        use_pushurl: args.use_pushurl.then_some(true),
        insecure,
        ..Default::default()
    };
    let mut app = App::new(work_dir, &cli_overrides)?;

    // Ensure remote exists (auto-create if possible)
    check_and_create_remote(&mut app)?;

    // Resolve branch via --track if no explicit branch given (before all mode dispatches)
    let branch = if args.track && args.branch.is_none() {
        match app.git.upstream_branch()? {
            Some((_remote, merge_branch)) => {
                tracing::debug!("--track resolved upstream branch to {}", merge_branch);
                Some(merge_branch)
            }
            None => None,
        }
    } else {
        args.branch.clone()
    };

    // Download mode
    if let Some(ref change_arg) = args.download {
        return review::cmd_review_download(&mut app, change_arg, &args.format).await;
    }

    // Cherry-pick modes
    if let Some(ref change_arg) = args.cherrypick {
        return review::cmd_review_cherrypick(&mut app, change_arg).await;
    }
    if let Some(ref change_arg) = args.cherrypickindicate {
        return review::cmd_review_cherrypickindicate(&mut app, change_arg).await;
    }
    if let Some(ref change_arg) = args.cherrypickonly {
        return review::cmd_review_cherrypickonly(&mut app, change_arg).await;
    }

    // Compare mode
    if let Some(ref compare_arg) = args.compare {
        let compare_branch = branch.as_deref().unwrap_or(&app.config.branch).to_string();
        return review::cmd_review_compare(
            &mut app,
            compare_arg,
            &compare_branch,
            args.no_rebase,
            args.force_rebase,
        )
        .await;
    }

    // List mode
    if args.list > 0 {
        return review::cmd_review_list(&mut app, branch.as_deref(), args.list >= 2, &args.format)
            .await;
    }

    // Pre-push: --update runs `git remote update`
    if args.update {
        let remote = args.remote.as_deref().unwrap_or(&app.config.remote);
        let root = app.git.root()?;
        tracing::info!("Updating remote {remote}...");
        subprocess::git_remote_update(remote, &root)?;
    }

    // Pre-push: --new-changeid strips Change-Id and amends
    if args.new_changeid {
        let root = app.git.root()?;
        tracing::info!("Regenerating Change-Id...");
        subprocess::git_regenerate_changeid(&root)?;
    }

    // Capture current branch name for --finish (only when not dry-run) (Task B2)
    let current_branch_name = if args.finish && !args.dry_run {
        Some(app.git.current_branch_or_default(&app.config.branch))
    } else {
        None
    };

    // Default topic to current branch name (Task H3)
    let topic = if args.no_topic {
        None
    } else {
        args.topic.clone().or_else(|| app.git.current_branch().ok())
    };

    // Default mode: push
    cmd_push(
        work_dir,
        PushArgs {
            branch,
            remote: args.remote.clone(),
            topic,
            wip: args.wip,
            ready: args.ready,
            private: args.private,
            remove_private: args.remove_private,
            reviewers: args.reviewers,
            cc: args.cc,
            hashtags: args.hashtags,
            message: args.message,
            notify: args.notify.map(|n| n.to_string()),
            no_rebase: args.no_rebase,
            force_rebase: args.force_rebase,
            keep_rebase: args.keep_rebase,
            dry_run: args.dry_run,
            yes: args.yes,
            new_changeid: false, // already handled above
            no_thin: args.no_thin,
            format: args.format.clone(),
            color_remote: color_remote.clone(),
        },
        insecure,
    )
    .await?;

    // Post-push: --finish checks out default branch and deletes topic branch (Task B2)
    if let Some(topic_branch) = current_branch_name {
        if !args.dry_run {
            let default_branch = app.config.branch.clone();
            let root = app.git.root()?;
            tracing::info!(
                "Finishing: checking out {} and deleting {}...",
                default_branch,
                topic_branch
            );
            subprocess::git_checkout(&default_branch, &root)?;
            subprocess::git_delete_branch(&topic_branch, &root)?;
        }
    }

    Ok(())
}

async fn cmd_push(work_dir: &Path, args: PushArgs, insecure: bool) -> Result<()> {
    let cli_overrides = CliOverrides {
        remote: args.remote.clone(),
        branch: args.branch.clone(),
        insecure,
        ..Default::default()
    };
    let mut app = App::new(work_dir, &cli_overrides)?;
    let root = app.git.root()?;

    // Ensure remote exists (auto-create if possible)
    check_and_create_remote(&mut app)?;

    // Ensure commit-msg hook is installed
    let hooks_dir = app.git.hooks_dir()?;
    hook::ensure_hook_installed(&hooks_dir)?;
    debug!("commit-msg hook verified at {:?}", hooks_dir);

    // Propagate hook to submodules (non-fatal)
    if let Err(e) = hook::propagate_hook_to_submodules(&root) {
        tracing::warn!("failed to propagate hook to submodules: {e}");
    }

    let branch = args.branch.unwrap_or_else(|| app.config.branch.clone());
    let remote = args.remote.unwrap_or_else(|| app.config.remote.clone());

    // Check Change-Id status with better error handling (Task M15)
    let commit_msg = app.git.head_commit_message()?;
    let hook_installed = app
        .git
        .hooks_dir()
        .map(|d| d.join("commit-msg").exists())
        .unwrap_or(false);
    match push::check_change_id_status(&commit_msg, hook_installed) {
        ChangeIdStatus::Present(_) => {}
        ChangeIdStatus::MissingCanAutoAmend => {
            eprintln!("No Change-Id found; amending commit to add one...");
            subprocess::git_exec(&["commit", "--amend", "--no-edit"], &root)?;
        }
        ChangeIdStatus::MissingNeedHook => {
            anyhow::bail!("HEAD commit is missing a Change-Id trailer. Run `grt setup` to install the commit-msg hook, then amend the commit");
        }
    }

    // Pre-push rebase (test rebase to detect conflicts)
    let should_rebase = !args.no_rebase && (app.config.default_rebase || args.force_rebase);
    let rebase_orig_head = if should_rebase {
        match rebase::rebase_changes(&remote, &branch, args.keep_rebase, &root)? {
            rebase::RebaseResult::Success { orig_head } => Some(orig_head),
            rebase::RebaseResult::Failed => return Ok(()),
            rebase::RebaseResult::Skipped => None,
        }
    } else {
        None
    };

    // Undo rebase BEFORE push (unless --force-rebase) so we push original SHAs.
    // This matches git-review's behavior: test rebase detects conflicts, but we
    // push the original commits so unchanged commits don't get new patchsets.
    if let Some(ref orig_head) = rebase_orig_head {
        if !args.force_rebase {
            rebase::undo_rebase(orig_head, &root)?;
        }
    }

    // Count unpushed commits
    let count = subprocess::count_unpushed_commits(&remote, &branch, &root)?;
    if count == 0 {
        eprintln!("No unpushed commits found.");
        return Ok(());
    }

    if count > 1 && !args.yes {
        let commits = subprocess::list_unpushed_commits(&remote, &branch, &root)?;
        eprintln!(
            "You are about to submit multiple commits to {remote}/{branch}:\n\n{commits}\n\nContinue? [y/N] "
        );
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
    };

    let refspec = push::build_refspec(&opts)?;

    let color_val = args.color_remote.as_deref().unwrap_or("always");
    let color_config = format!("color.remote={}", color_val);

    // Dry-run: show full command with all flags (Task L13)
    if args.dry_run {
        let mut dry_args: Vec<&str> = vec!["git", "-c", &color_config, "push", "--no-follow-tags"];
        if args.no_thin {
            dry_args.push("--no-thin");
        }
        dry_args.push(&remote);
        dry_args.push(&refspec);
        println!("{}", dry_args.join(" "));
        return Ok(());
    }

    // Build push args with --no-follow-tags, color remote, and optional --no-thin
    // (Tasks M6, M7, L13, L15)
    let mut push_args: Vec<&str> = vec!["-c", &color_config, "push", "--no-follow-tags"];
    if args.no_thin {
        push_args.push("--no-thin");
    }
    push_args.push(&remote);
    push_args.push(&refspec);

    eprintln!("Pushing {count} commit(s) to {remote}/{branch}...");

    // Catch push errors and suggest --no-thin for "Missing tree" (Task L14)
    if let Err(e) = subprocess::git_exec(&push_args, &root) {
        let msg = format!("{e:#}");
        if msg.contains("Missing tree") || msg.contains("missing tree") {
            eprintln!("hint: Consider trying again with --no-thin");
        }
        return Err(e);
    }

    match args.format {
        OutputFormat::Json => {
            // Re-read commit message to get Change-Id (may have been added by amend)
            let commit_msg = app.git.head_commit_message().unwrap_or_default();
            let result = PushResult {
                commits: count,
                remote: remote.clone(),
                branch: branch.clone(),
                change_id: push::extract_change_id(&commit_msg),
                refspec: refspec.clone(),
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text => {
            eprintln!("Push successful.");
        }
    }

    Ok(())
}

async fn cmd_comments(work_dir: &Path, args: CommentsArgs, insecure: bool) -> Result<()> {
    let cli_overrides = CliOverrides {
        insecure,
        ..Default::default()
    };
    let mut app = App::new(work_dir, &cli_overrides)?;
    app.authenticate_and_verify().await?;

    // Resolve --age / --max-age into YYYY-MM-DD date bounds used by both modes.
    // --age N   → keep threads newer than N ago  → `after` lower bound
    // --max-age N → keep threads older than N ago → `before` upper bound
    let age_after: Option<String> = args
        .age
        .as_deref()
        .map(parse_age_to_date)
        .transpose()?;
    let age_before: Option<String> = args
        .min_age
        .as_deref()
        .map(parse_age_to_date)
        .transpose()?;

    // Search mode: no change given, but --project or --age provided
    let search_mode = args.change.is_none()
        && (args.project.is_some() || args.age.is_some());

    if search_mode {
        // Require at least one server-side filter
        if args.project.is_none() && args.age.is_none() {
            anyhow::bail!("cross-change search requires --project and/or --age");
        }

        let mut query_parts = Vec::new();
        if let Some(ref proj) = args.project {
            query_parts.push(format!("project:{proj}"));
        }
        if let Some(ref age) = args.age {
            // -age:N = changes active within the last N (newer than N ago)
            query_parts.push(format!("-age:{age}"));
        }
        if let Some(ref min_age) = args.min_age {
            // age:N = changes last modified more than N ago (older than N ago)
            query_parts.push(format!("age:{min_age}"));
        }
        // Use commentby: for server-side pre-filtering — unlike reviewer: it matches
        // anyone who has posted a comment, including CI bots that are not formal reviewers.
        if let Some(ref pat) = args.comment_by {
            query_parts.push(format!("commentby:{pat}"));
        }
        let query = query_parts.join(" ");

        let changes = app.gerrit.query_changes(&query).await?;
        let gerrit_url = app.config.gerrit_base_url()?.to_string();

        let mut outputs: Vec<comments::CommentOutput> = Vec::new();

        for change in &changes {
            let change_id = match change.number {
                Some(n) => n.to_string(),
                None => continue,
            };

            let change_detail = app.gerrit.get_change_detail(&change_id).await?;
            // In search mode always fetch all revisions — a commenter may have
            // reviewed an earlier patchset that is no longer the current one.
            let mut all_comments = app.gerrit.get_change_comments(&change_id).await?;

            // Always include robot comments in search mode: CI bots comment via
            // the robot comments endpoint and would be invisible otherwise.
            if let Ok(robot) = app.gerrit.get_robot_comments(&change_id).await {
                for (file, rc) in robot {
                    all_comments.entry(file).or_default().extend(rc);
                }
            }

            // Apply --comment-by filter on raw map
            if let Some(ref pat) = args.comment_by {
                comments::filter_by_author(&mut all_comments, pat);
            }

            let mut threads = comments::build_threads(&all_comments);

            // Apply filters.
            // In search mode --age drives the Gerrit query (change activity window),
            // so do NOT apply it as a per-comment date filter here.  Use explicit
            // --after / --before for per-comment date filtering in search mode.
            if args.has_replies {
                comments::filter_threads_has_replies(&mut threads);
            }
            comments::filter_threads_by_date(
                &mut threads,
                args.after.as_deref(),
                args.before.as_deref(),
            );

            // Label filter
            if let Some(ref label_arg) = args.label {
                apply_label_filter(&change_detail, label_arg, &mut threads)?;
            }

            if args.unresolved {
                threads.retain(|t| !t.resolved);
            }

            if threads.is_empty() {
                continue; // all threads filtered out for this change
            }

            let messages = change_detail.messages.as_deref().unwrap_or(&[]);
            match args.format {
                OutputFormat::Json => {
                    let json = comments::format_json(&change_detail, messages, &threads, &gerrit_url);
                    if let Ok(output) = serde_json::from_value(json) {
                        outputs.push(output);
                    }
                }
                OutputFormat::Text => {
                    let text = comments::format_text(&change_detail, messages, &threads, &gerrit_url);
                    print!("{text}");
                    println!("\n---\n");
                }
            }
        }

        if matches!(args.format, OutputFormat::Json) {
            let multi = comments::format_json_multi(&outputs);
            println!("{}", serde_json::to_string_pretty(&multi)?);
        }

        return Ok(());
    }

    // Single-change mode
    let change_id = match args.change {
        Some(id) => id,
        None => {
            let msg = app.git.head_commit_message()?;
            push::extract_change_id(&msg)
                .context("no Change-Id found in HEAD commit. Specify a change number explicitly")?
        }
    };

    debug!("fetching comments for change: {}", change_id);

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

    if args.include_robot_comments {
        if let Ok(robot) = app.gerrit.get_robot_comments(&change_id).await {
            for (file, robot_comments) in robot {
                all_comments.entry(file).or_default().extend(robot_comments);
            }
        }
    }

    // Apply --comment-by filter on raw map (before build_threads)
    if let Some(ref pat) = args.comment_by {
        comments::filter_by_author(&mut all_comments, pat);
    }

    let mut threads = comments::build_threads(&all_comments);

    // Apply filters
    if args.has_replies {
        comments::filter_threads_has_replies(&mut threads);
    }
    let after = args.after.as_deref().or(age_after.as_deref());
    let before = args.before.as_deref().or(age_before.as_deref());
    comments::filter_threads_by_date(&mut threads, after, before);

    // Label filter
    if let Some(ref label_arg) = args.label {
        apply_label_filter(&change, label_arg, &mut threads)?;
    }

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

/// Resolve the transport scheme for `grt setup`.
///
/// SSH is the default and is used whenever `--http` is not passed.
/// `--ssh` is accepted for explicitness but has the same effect as omitting both flags.
fn setup_scheme(_ssh: bool, http: bool) -> &'static str {
    if http { "https" } else { "ssh" }
}

/// Return true when setup should run HTTP connectivity and auth checks.
/// SSH-only setup skips REST API calls to avoid prompting for HTTPS credentials.
fn setup_needs_http_check(scheme: &str) -> bool {
    scheme != "ssh"
}

/// Parse an age string like "1d", "2w", "3m", "1y" into a YYYY-MM-DD date
/// representing that many days/weeks/months/years before today.
///
/// Returned date can be used as an `after` or `before` bound with
/// `filter_threads_by_date`.
fn parse_age_to_date(age: &str) -> Result<String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let (count_str, unit) = age
        .find(|c: char| c.is_alphabetic())
        .map(|i| (&age[..i], &age[i..]))
        .context("age must be a number followed by a unit (d, w, m, y)")?;
    let count: u64 = count_str
        .parse()
        .context("age count must be a positive integer")?;
    if count == 0 {
        anyhow::bail!("age count must be greater than zero");
    }

    let days: u64 = match unit {
        "d" => count,
        "w" => count * 7,
        "m" => count * 30,
        "y" => count * 365,
        other => anyhow::bail!("unknown age unit '{}'; use d, w, m, or y", other),
    };

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before Unix epoch")?
        .as_secs();
    let cutoff_secs = now_secs.saturating_sub(days * 86_400);

    // Convert epoch seconds to YYYY-MM-DD without external deps
    let date = epoch_secs_to_date(cutoff_secs);
    Ok(date)
}

/// Convert Unix epoch seconds to a "YYYY-MM-DD" string using the proleptic
/// Gregorian calendar.  Accurate for dates in the range 1970–9999.
fn epoch_secs_to_date(secs: u64) -> String {
    let days_since_epoch = secs / 86_400;
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    // civil_from_days (public domain)
    let z = days_since_epoch as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Parse a label filter like "Code-Review=-1" and retain only threads
/// where at least one commenter's account_id voted that label value.
fn apply_label_filter(
    change: &grt::gerrit::ChangeInfo,
    label_arg: &str,
    threads: &mut Vec<grt::comments::CommentThread>,
) -> Result<()> {
    let (label_name, value_str) = label_arg
        .split_once('=')
        .context("--label must be in format 'LabelName=value' (e.g., Code-Review=-1)")?;
    let target_value: i32 = value_str
        .parse()
        .context("label value must be an integer")?;

    let voter_ids: std::collections::HashSet<i64> = change
        .labels
        .as_ref()
        .and_then(|labels| labels.get(label_name))
        .and_then(|info| info.all.as_ref())
        .map(|approvals| {
            approvals
                .iter()
                .filter(|a| a.value == Some(target_value))
                .filter_map(|a| a.account_id)
                .collect()
        })
        .unwrap_or_default();

    if !voter_ids.is_empty() {
        threads.retain(|t| {
            t.comments
                .iter()
                .any(|c| c.account_id.map(|id| voter_ids.contains(&id)).unwrap_or(false))
        });
    }

    Ok(())
}

async fn cmd_setup(work_dir: &Path, args: SetupArgs, insecure: bool) -> Result<()> {
    let scheme = Some(setup_scheme(args.ssh, args.http).to_string());

    let cli_overrides = CliOverrides {
        remote: args.remote.clone(),
        scheme,
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
    if args.remote_hook {
        // Download hook from remote Gerrit server (HTTP or SCP based on remote URL)
        let remote_name = args.remote.as_deref().unwrap_or(&app.config.remote);
        let remote_url = review_query::resolve_remote_url(
            remote_name,
            &root,
            Some(&app.config.make_remote_url()),
        )?
        .or_else(|| app.config.gerrit_base_url().ok().map(|u| u.to_string()))
        .context("no remote URL for hook download")?;
        hook::fetch_remote_hook(&remote_url, &hooks_dir).await?;
    } else {
        hook::ensure_hook_installed(&hooks_dir)?;
        eprintln!("  commit-msg hook: installed at {}", hook_path.display());
    }

    // Propagate hook to submodules
    if let Err(e) = hook::propagate_hook_to_submodules(&root) {
        tracing::warn!("failed to propagate hook to submodules: {e}");
    }

    // 2. Ensure remote exists (create if missing, prompt for SSH username if needed)
    check_and_create_remote(&mut app)?;
    let remote = args.remote.unwrap_or_else(|| app.config.remote.clone());
    match subprocess::git_output(&["remote", "get-url", &remote], &root) {
        Ok(url) => eprintln!("  remote '{remote}': {}", url.trim()),
        Err(_) => eprintln!("  remote '{remote}': NOT FOUND"),
    }

    // 3. Test connectivity and auth (HTTP/HTTPS only — skip for SSH-only setup)
    let use_http = setup_needs_http_check(&app.config.scheme);
    if use_http {
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

    // === Completions subcommand ===

    #[test]
    fn parse_completions_bash() {
        let cli = Cli::parse_from(["grt", "completions", "bash"]);
        assert!(matches!(
            cli.command,
            Commands::Completions {
                shell: clap_complete::Shell::Bash
            }
        ));
    }

    #[test]
    fn parse_completions_zsh() {
        let cli = Cli::parse_from(["grt", "completions", "zsh"]);
        assert!(matches!(
            cli.command,
            Commands::Completions {
                shell: clap_complete::Shell::Zsh
            }
        ));
    }

    #[test]
    fn parse_completions_fish() {
        let cli = Cli::parse_from(["grt", "completions", "fish"]);
        assert!(matches!(
            cli.command,
            Commands::Completions {
                shell: clap_complete::Shell::Fish
            }
        ));
    }

    // === Exit code mapping ===

    #[test]
    fn exit_code_network_error() {
        let err: anyhow::Error = GerritError::Network("connection refused".into()).into();
        assert_eq!(exit_code_for_error(&err), 40);
    }

    #[test]
    fn exit_code_auth_error() {
        let err: anyhow::Error = GerritError::AuthFailed { status: 401 }.into();
        assert_eq!(exit_code_for_error(&err), 1);
    }

    #[test]
    fn exit_code_generic() {
        let err = anyhow::anyhow!("something went wrong");
        assert_eq!(exit_code_for_error(&err), 1);
    }

    #[test]
    fn exit_code_config_error() {
        let err = anyhow::anyhow!("no Gerrit host configured");
        assert_eq!(exit_code_for_error(&err), 128);
    }

    #[test]
    fn exit_code_malformed_input() {
        let err = anyhow::anyhow!("compare argument must be CHANGE,PS[-PS]");
        assert_eq!(exit_code_for_error(&err), 3);
    }

    // === Task B2: --finish guard on dry_run ===

    #[test]
    fn parse_review_finish_flag() {
        let cli = Cli::parse_from(["grt", "review", "-f"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.finish);
            assert!(!args.dry_run);
        } else {
            panic!("expected Review command");
        }
    }

    #[test]
    fn parse_review_finish_with_dry_run() {
        let cli = Cli::parse_from(["grt", "review", "-f", "-n"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.finish);
            assert!(args.dry_run);
        } else {
            panic!("expected Review command");
        }
    }

    // === Task H2: force_rebase flag parsing ===

    #[test]
    fn parse_review_force_rebase() {
        let cli = Cli::parse_from(["grt", "review", "-F"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.force_rebase);
        } else {
            panic!("expected Review command");
        }
    }

    // === Task H3: default topic to branch name ===

    #[test]
    fn parse_review_no_topic_flag() {
        let cli = Cli::parse_from(["grt", "review", "-T"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.no_topic);
        } else {
            panic!("expected Review command");
        }
    }

    // === Task M7: no_thin flag in PushArgs ===

    #[test]
    fn parse_push_no_thin() {
        let cli = Cli::parse_from(["grt", "push", "--no-thin"]);
        if let Commands::Push(args) = cli.command {
            assert!(args.no_thin);
        } else {
            panic!("expected Push command");
        }
    }

    #[test]
    fn parse_push_no_thin_default_false() {
        let cli = Cli::parse_from(["grt", "push"]);
        if let Commands::Push(args) = cli.command {
            assert!(!args.no_thin);
        } else {
            panic!("expected Push command");
        }
    }

    // === Task M14: --color/--no-color warnings ===

    #[test]
    fn git_review_parse_color_flag() {
        let cli = GitReviewCli::parse_from(["git-review", "--color", "always", "main"]);
        assert_eq!(cli.color.as_deref(), Some("always"));
    }

    // === Task M16: --setup --finish together ===

    #[test]
    fn parse_review_setup_and_finish() {
        // setup is in the "mode" group, but finish is not, so they can coexist
        let cli = Cli::parse_from(["grt", "review", "-s", "-f"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.setup);
            assert!(args.finish);
        } else {
            panic!("expected Review command");
        }
    }

    // === Task L2: --license flag ===

    #[test]
    fn git_review_parse_license() {
        let cli = GitReviewCli::parse_from(["git-review", "--license"]);
        assert!(cli.license);
    }

    #[test]
    fn git_review_license_default_false() {
        let cli = GitReviewCli::parse_from(["git-review", "main"]);
        assert!(!cli.license);
    }

    // === Task L13: dry-run shows full push command ===

    #[test]
    fn parse_push_dry_run_flag() {
        let cli = Cli::parse_from(["grt", "push", "--dry-run"]);
        if let Commands::Push(args) = cli.command {
            assert!(args.dry_run);
        } else {
            panic!("expected Push command");
        }
    }

    // === Task M7: no_thin threading through review ===

    #[test]
    fn parse_review_no_thin() {
        let cli = Cli::parse_from(["grt", "review", "--no-thin"]);
        if let Commands::Review(args) = cli.command {
            assert!(args.no_thin);
        } else {
            panic!("expected Review command");
        }
    }

    // === setup_scheme ===

    #[test]
    fn setup_scheme_no_flags_defaults_to_ssh() {
        assert_eq!(setup_scheme(false, false), "ssh");
    }

    #[test]
    fn setup_scheme_explicit_ssh_flag_is_ssh() {
        assert_eq!(setup_scheme(true, false), "ssh");
    }

    #[test]
    fn setup_scheme_http_flag_is_https() {
        assert_eq!(setup_scheme(false, true), "https");
    }

    #[test]
    fn parse_setup_defaults_to_no_flags() {
        let cli = Cli::parse_from(["grt", "setup"]);
        if let Commands::Setup(args) = cli.command {
            assert!(!args.ssh);
            assert!(!args.http);
            assert_eq!(setup_scheme(args.ssh, args.http), "ssh");
        } else {
            panic!("expected Setup command");
        }
    }

    #[test]
    fn parse_setup_ssh_flag() {
        let cli = Cli::parse_from(["grt", "setup", "--ssh"]);
        if let Commands::Setup(args) = cli.command {
            assert!(args.ssh);
            assert!(!args.http);
            assert_eq!(setup_scheme(args.ssh, args.http), "ssh");
        } else {
            panic!("expected Setup command");
        }
    }

    #[test]
    fn parse_setup_http_flag() {
        let cli = Cli::parse_from(["grt", "setup", "--http"]);
        if let Commands::Setup(args) = cli.command {
            assert!(!args.ssh);
            assert!(args.http);
            assert_eq!(setup_scheme(args.ssh, args.http), "https");
        } else {
            panic!("expected Setup command");
        }
    }

    #[test]
    fn parse_setup_ssh_and_http_conflict() {
        // clap should reject --ssh and --http together
        let result = Cli::try_parse_from(["grt", "setup", "--ssh", "--http"]);
        assert!(result.is_err(), "--ssh and --http should conflict");
    }

    // === setup_needs_http_check ===

    #[test]
    fn http_check_skipped_for_ssh_scheme() {
        assert!(!setup_needs_http_check("ssh"));
    }

    #[test]
    fn http_check_runs_for_https_scheme() {
        assert!(setup_needs_http_check("https"));
    }

    #[test]
    fn http_check_runs_for_http_scheme() {
        assert!(setup_needs_http_check("http"));
    }

    #[test]
    fn no_flag_setup_scheme_skips_http_check() {
        // Default (no flags) must be SSH and must NOT trigger HTTP checks
        let scheme = setup_scheme(false, false);
        assert_eq!(scheme, "ssh");
        assert!(!setup_needs_http_check(scheme));
    }

    #[test]
    fn explicit_ssh_flag_skips_http_check() {
        let scheme = setup_scheme(true, false);
        assert_eq!(scheme, "ssh");
        assert!(!setup_needs_http_check(scheme));
    }

    #[test]
    fn http_flag_enables_http_check() {
        let scheme = setup_scheme(false, true);
        assert_eq!(scheme, "https");
        assert!(setup_needs_http_check(scheme));
    }

    // === age semantics: --age = max age (≤ N old), --min-age = min age (≥ N old) ===

    #[test]
    fn age_flag_is_after_bound() {
        // --age 1d → cutoff = yesterday; keep threads newer than or equal to that date
        let cutoff = parse_age_to_date("1d").unwrap();
        let mut threads = vec![comments::CommentThread {
            file: "f".into(),
            line: None,
            resolved: false,
            comments: vec![comments::ThreadComment {
                author: "A".into(),
                author_email: None,
                account_id: None,
                patch_set: None,
                date: format!("{} 00:00:00.000000000", cutoff),
                message: "msg".into(),
            }],
        }];
        comments::filter_threads_by_date(&mut threads, Some(&cutoff), None);
        assert_eq!(threads.len(), 1, "--age boundary date should be kept");
    }

    // === epoch_secs_to_date ===

    #[test]
    fn epoch_secs_to_date_unix_epoch() {
        assert_eq!(epoch_secs_to_date(0), "1970-01-01");
    }

    #[test]
    fn epoch_secs_to_date_known_date() {
        // 2025-01-01 00:00:00 UTC = 1735689600
        assert_eq!(epoch_secs_to_date(1_735_689_600), "2025-01-01");
    }

    #[test]
    fn epoch_secs_to_date_leap_day() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        assert_eq!(epoch_secs_to_date(1_709_164_800), "2024-02-29");
    }

    // === parse_age_to_date ===

    #[test]
    fn parse_age_days() {
        // Just verify the function doesn't error and returns a plausible date string
        let date = parse_age_to_date("7d").unwrap();
        assert_eq!(date.len(), 10);
        assert!(date.starts_with("20"));
    }

    #[test]
    fn parse_age_weeks() {
        let date = parse_age_to_date("2w").unwrap();
        assert_eq!(date.len(), 10);
    }

    #[test]
    fn parse_age_months() {
        let date = parse_age_to_date("1m").unwrap();
        assert_eq!(date.len(), 10);
    }

    #[test]
    fn parse_age_years() {
        let date = parse_age_to_date("1y").unwrap();
        assert_eq!(date.len(), 10);
    }

    #[test]
    fn parse_age_invalid_unit() {
        assert!(parse_age_to_date("5x").is_err());
    }

    #[test]
    fn parse_age_zero_count() {
        assert!(parse_age_to_date("0d").is_err());
    }

    #[test]
    fn parse_age_no_unit() {
        assert!(parse_age_to_date("30").is_err());
    }

    #[test]
    fn parse_age_days_older_than_weeks() {
        // 30d should produce an earlier date than 1d
        let date_30d = parse_age_to_date("30d").unwrap();
        let date_1d = parse_age_to_date("1d").unwrap();
        assert!(date_30d < date_1d, "30d cutoff should be earlier than 1d cutoff");
    }
}
