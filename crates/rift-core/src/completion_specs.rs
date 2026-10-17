//! Command specs for rich completions (subcommands, flags, generators).
//!
//! Fully offline: a compact embedded table of popular commands plus a handful
//! of cheap, synchronous, filesystem-only generators (git refs/remotes, ssh
//! hosts, make targets, package.json scripts). Nothing here shells out, spawns
//! a process, or blocks; every generator is bounded and never panics.
//!
//! `completion.rs` owns the `Completion` type and does the wiring; this module
//! is pure data + lookup/resolution + generators + a tiny shell tokenizer.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a positional argument at some spec level should be completed against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    None,
    /// Any filesystem path (files + dirs).
    Path,
    /// Directories only (e.g. `cd`).
    Dir,
    /// Any git ref: local branch, remote branch, or tag.
    GitRef,
    /// Local branches only.
    GitBranch,
    /// Configured git remotes.
    GitRemote,
    /// Hosts from ssh config / known_hosts.
    SshHost,
    /// Targets in a Makefile in the cwd.
    MakeTarget,
    /// Scripts in a package.json in the cwd.
    PkgScript,
}

/// A single command-line option / flag.
#[derive(Debug, Clone, Copy)]
pub struct OptionSpec {
    /// Short form incl. dash, e.g. `-m`. Empty = none.
    pub short: &'static str,
    /// Long form incl. dashes, e.g. `--message`. Empty = none.
    pub long: &'static str,
    pub desc: &'static str,
    /// Whether the option consumes the following word as its value.
    pub takes_arg: bool,
    /// Whether the option may appear more than once (skip dedup).
    pub repeatable: bool,
}

/// A subcommand, which may itself nest subcommands.
#[derive(Debug, Clone, Copy)]
pub struct SubcommandSpec {
    pub name: &'static str,
    pub desc: &'static str,
    pub options: &'static [OptionSpec],
    pub subcommands: &'static [SubcommandSpec],
    pub args: ArgKind,
}

/// A top-level command.
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub options: &'static [OptionSpec],
    pub subcommands: &'static [SubcommandSpec],
    pub args: ArgKind,
}

// --- terse const builders -------------------------------------------------

const fn o(short: &'static str, long: &'static str, desc: &'static str) -> OptionSpec {
    OptionSpec {
        short,
        long,
        desc,
        takes_arg: false,
        repeatable: false,
    }
}
const fn oa(short: &'static str, long: &'static str, desc: &'static str) -> OptionSpec {
    OptionSpec {
        short,
        long,
        desc,
        takes_arg: true,
        repeatable: false,
    }
}
const fn oar(short: &'static str, long: &'static str, desc: &'static str) -> OptionSpec {
    OptionSpec {
        short,
        long,
        desc,
        takes_arg: true,
        repeatable: true,
    }
}

const fn sc(name: &'static str, desc: &'static str) -> SubcommandSpec {
    SubcommandSpec {
        name,
        desc,
        options: &[],
        subcommands: &[],
        args: ArgKind::None,
    }
}
const fn sca(name: &'static str, desc: &'static str, args: ArgKind) -> SubcommandSpec {
    SubcommandSpec {
        name,
        desc,
        options: &[],
        subcommands: &[],
        args,
    }
}
const fn sco(
    name: &'static str,
    desc: &'static str,
    options: &'static [OptionSpec],
) -> SubcommandSpec {
    SubcommandSpec {
        name,
        desc,
        options,
        subcommands: &[],
        args: ArgKind::None,
    }
}
const fn scoa(
    name: &'static str,
    desc: &'static str,
    options: &'static [OptionSpec],
    args: ArgKind,
) -> SubcommandSpec {
    SubcommandSpec {
        name,
        desc,
        options,
        subcommands: &[],
        args,
    }
}
const fn scn(
    name: &'static str,
    desc: &'static str,
    subcommands: &'static [SubcommandSpec],
) -> SubcommandSpec {
    SubcommandSpec {
        name,
        desc,
        options: &[],
        subcommands,
        args: ArgKind::None,
    }
}

// ==========================================================================
// git
// ==========================================================================

const GIT_GLOBAL: &[OptionSpec] = &[
    oa("-C", "", "Run as if git was started in <path>"),
    oa("-c", "", "Pass a configuration parameter to the command"),
    o("-p", "--paginate", "Pipe all output into a pager"),
    o("", "--no-pager", "Do not pipe output into a pager"),
    o("", "--version", "Show the git version"),
    o("", "--help", "Show help"),
];

const GIT_ADD: &[OptionSpec] = &[
    o(
        "-A",
        "--all",
        "Add changes from all tracked and untracked files",
    ),
    o("-p", "--patch", "Interactively choose hunks to stage"),
    o("-f", "--force", "Allow adding otherwise ignored files"),
    o("-u", "--update", "Update tracked files only"),
    o("-n", "--dry-run", "Show what would be added"),
    o("-v", "--verbose", "Be verbose"),
];
const GIT_COMMIT: &[OptionSpec] = &[
    oar("-m", "--message", "Use the given commit message"),
    o(
        "-a",
        "--all",
        "Stage modified and deleted files automatically",
    ),
    o("", "--amend", "Amend the previous commit"),
    o("", "--no-edit", "Reuse the commit message without editing"),
    o("-s", "--signoff", "Add a Signed-off-by trailer"),
    o(
        "-v",
        "--verbose",
        "Show diff in the commit message template",
    ),
    o("", "--no-verify", "Bypass pre-commit and commit-msg hooks"),
    oa(
        "-C",
        "--reuse-message",
        "Reuse message from the given commit",
    ),
];
const GIT_CHECKOUT: &[OptionSpec] = &[
    oa("-b", "", "Create and switch to a new branch"),
    oa("-B", "", "Create/reset and switch to a branch"),
    o("-t", "--track", "Set up tracking mode"),
    o("-f", "--force", "Force checkout (throw away local changes)"),
    o("", "--detach", "Detach HEAD at the named commit"),
    o("", "--orphan", "Create a new orphan branch"),
];
const GIT_SWITCH: &[OptionSpec] = &[
    oa("-c", "--create", "Create and switch to a new branch"),
    oa(
        "-C",
        "--force-create",
        "Create/reset and switch to a branch",
    ),
    o(
        "-d",
        "--detach",
        "Switch to a commit in detached HEAD state",
    ),
    o("-f", "--force", "Discard local changes when switching"),
    o("", "--track", "Set up tracking mode"),
];
const GIT_BRANCH: &[OptionSpec] = &[
    o("-d", "--delete", "Delete a branch"),
    o("-D", "", "Force-delete a branch"),
    o("-m", "--move", "Rename a branch"),
    o("-a", "--all", "List both remote and local branches"),
    o("-r", "--remotes", "List or delete remote-tracking branches"),
    o(
        "-v",
        "--verbose",
        "Show sha1 and subject line for each head",
    ),
    o("-l", "--list", "List branches"),
    o("-f", "--force", "Reset branch even if it exists"),
    o("", "--merged", "List branches merged into HEAD"),
    o("", "--no-merged", "List branches not merged into HEAD"),
];
const GIT_PUSH: &[OptionSpec] = &[
    o("-u", "--set-upstream", "Set upstream for the pushed branch"),
    o("-f", "--force", "Force update the remote ref"),
    o(
        "",
        "--force-with-lease",
        "Force push only if the remote is unchanged",
    ),
    o("", "--tags", "Push all tags"),
    o("", "--all", "Push all branches"),
    o("-d", "--delete", "Delete the given remote ref"),
    o("-n", "--dry-run", "Do everything except actually send"),
    o("-v", "--verbose", "Be verbose"),
];
const GIT_PULL: &[OptionSpec] = &[
    o(
        "",
        "--rebase",
        "Rebase local commits on top of the upstream",
    ),
    o(
        "",
        "--no-rebase",
        "Merge the upstream into the current branch",
    ),
    o("", "--ff-only", "Refuse to merge unless fast-forward"),
    o(
        "",
        "--no-ff",
        "Create a merge commit even for a fast-forward",
    ),
    o(
        "-n",
        "--dry-run",
        "Do everything except actually fetch/merge",
    ),
    o("-v", "--verbose", "Be verbose"),
];
const GIT_FETCH: &[OptionSpec] = &[
    o("", "--all", "Fetch all remotes"),
    o(
        "-p",
        "--prune",
        "Remove remote-tracking refs that no longer exist",
    ),
    o("", "--tags", "Fetch all tags"),
    o("-n", "--dry-run", "Show what would be done"),
    o("-v", "--verbose", "Be verbose"),
];
const GIT_LOG: &[OptionSpec] = &[
    o("", "--oneline", "Show each commit as a single line"),
    o("", "--graph", "Draw a text-based commit graph"),
    o("-p", "--patch", "Show the diff introduced by each commit"),
    o("", "--stat", "Show a diffstat for each commit"),
    o("", "--all", "Show commits from all refs"),
    o("", "--decorate", "Show ref names on shown commits"),
    oa("-n", "", "Limit the number of commits shown"),
    oa("", "--author", "Limit to commits by the given author"),
    oa("", "--since", "Show commits more recent than a date"),
    oa("", "--until", "Show commits older than a date"),
];
const GIT_RESET: &[OptionSpec] = &[
    o("", "--soft", "Keep index and working tree, move HEAD only"),
    o("", "--mixed", "Reset index but not working tree (default)"),
    o("", "--hard", "Reset index and working tree"),
    o("", "--keep", "Reset but keep local changes where possible"),
    o("-p", "--patch", "Interactively select hunks to reset"),
];
const GIT_REBASE: &[OptionSpec] = &[
    o("-i", "--interactive", "Edit the list of commits to rebase"),
    oa("", "--onto", "Rebase onto the given base"),
    o("", "--continue", "Continue after resolving conflicts"),
    o("", "--abort", "Abort and reset to the original branch"),
    o("", "--skip", "Skip the current patch and continue"),
    o("", "--autosquash", "Auto-order fixup!/squash! commits"),
];
const GIT_STASH_SUBS: &[SubcommandSpec] = &[
    sc("push", "Save your local modifications to a new stash"),
    sc("pop", "Apply and remove the latest stash"),
    sc("apply", "Apply a stash without removing it"),
    sc("list", "List the stashes you currently have"),
    sc("show", "Show the changes recorded in a stash"),
    sc("drop", "Remove a single stash entry"),
    sc("branch", "Create a branch from a stash"),
    sc("clear", "Remove all stash entries"),
];
const GIT_DIFF: &[OptionSpec] = &[
    o("", "--staged", "Show staged changes"),
    o("", "--cached", "Show staged changes (alias of --staged)"),
    o("", "--stat", "Show a diffstat instead of a patch"),
    o("", "--name-only", "Show only names of changed files"),
    o(
        "",
        "--name-status",
        "Show names and status of changed files",
    ),
    o(
        "-w",
        "--ignore-all-space",
        "Ignore whitespace when comparing",
    ),
    o("", "--color", "Colorize the diff"),
];
const GIT_CLONE: &[OptionSpec] = &[
    oa("", "--depth", "Create a shallow clone of N commits"),
    oa("-b", "--branch", "Check out the given branch after cloning"),
    o("", "--recursive", "Initialize submodules recursively"),
    o("", "--bare", "Make a bare repository"),
    o("-q", "--quiet", "Operate quietly"),
];
const GIT_MERGE: &[OptionSpec] = &[
    o("", "--no-ff", "Always create a merge commit"),
    o("", "--ff-only", "Refuse to merge unless fast-forward"),
    o("", "--squash", "Squash the merged commits into the index"),
    o("", "--abort", "Abort the current merge"),
    o("", "--continue", "Continue after resolving conflicts"),
    oar("-m", "--message", "Set the merge commit message"),
];
const GIT_STATUS: &[OptionSpec] = &[
    o("-s", "--short", "Give output in the short format"),
    o("-b", "--branch", "Show the branch and tracking info"),
    o("", "--porcelain", "Machine-readable output"),
    o("", "--ignored", "Show ignored files as well"),
];
const GIT_TAG: &[OptionSpec] = &[
    o("-a", "--annotate", "Create an annotated tag"),
    oar("-m", "--message", "Set the tag message"),
    o("-d", "--delete", "Delete the given tag"),
    o("-l", "--list", "List tags"),
    o("-f", "--force", "Replace an existing tag"),
];
const GIT_RESTORE: &[OptionSpec] = &[
    o("", "--staged", "Restore the index"),
    o("-W", "--worktree", "Restore the working tree (default)"),
    oa("-s", "--source", "Restore from the given tree/commit"),
    o("-p", "--patch", "Interactively select hunks to restore"),
];
const GIT_REMOTE_SUBS: &[SubcommandSpec] = &[
    sc("add", "Add a remote"),
    sc("remove", "Remove a remote"),
    sc("rename", "Rename a remote"),
    sc("set-url", "Change the URL of a remote"),
    sco(
        "-v",
        "Show remotes with URLs",
        &[o("-v", "--verbose", "Show URLs")],
    ),
    sc("show", "Show information about a remote"),
    sc("prune", "Delete stale remote-tracking branches"),
];

const GIT_SUBS: &[SubcommandSpec] = &[
    scoa(
        "add",
        "Add file contents to the index",
        GIT_ADD,
        ArgKind::Path,
    ),
    scoa(
        "commit",
        "Record changes to the repository",
        GIT_COMMIT,
        ArgKind::Path,
    ),
    scoa(
        "checkout",
        "Switch branches or restore working tree files",
        GIT_CHECKOUT,
        ArgKind::GitRef,
    ),
    scoa("switch", "Switch branches", GIT_SWITCH, ArgKind::GitBranch),
    scoa(
        "branch",
        "List, create, or delete branches",
        GIT_BRANCH,
        ArgKind::GitBranch,
    ),
    scoa(
        "push",
        "Update remote refs along with objects",
        GIT_PUSH,
        ArgKind::GitRemote,
    ),
    scoa(
        "pull",
        "Fetch and integrate with another repository",
        GIT_PULL,
        ArgKind::GitRemote,
    ),
    scoa(
        "fetch",
        "Download objects and refs from another repository",
        GIT_FETCH,
        ArgKind::GitRemote,
    ),
    scoa("log", "Show commit logs", GIT_LOG, ArgKind::GitRef),
    scoa(
        "reset",
        "Reset current HEAD to the specified state",
        GIT_RESET,
        ArgKind::GitRef,
    ),
    scoa(
        "rebase",
        "Reapply commits on top of another base tip",
        GIT_REBASE,
        ArgKind::GitRef,
    ),
    SubcommandSpec {
        name: "stash",
        desc: "Stash changes in a dirty working directory",
        options: &[],
        subcommands: GIT_STASH_SUBS,
        args: ArgKind::None,
    },
    scoa(
        "diff",
        "Show changes between commits and working tree",
        GIT_DIFF,
        ArgKind::GitRef,
    ),
    scoa(
        "clone",
        "Clone a repository into a new directory",
        GIT_CLONE,
        ArgKind::None,
    ),
    scoa(
        "merge",
        "Join two or more development histories together",
        GIT_MERGE,
        ArgKind::GitBranch,
    ),
    scoa(
        "status",
        "Show the working tree status",
        GIT_STATUS,
        ArgKind::None,
    ),
    scoa(
        "tag",
        "Create, list, delete or verify tags",
        GIT_TAG,
        ArgKind::None,
    ),
    scoa(
        "restore",
        "Restore working tree files",
        GIT_RESTORE,
        ArgKind::Path,
    ),
    scn(
        "remote",
        "Manage set of tracked repositories",
        GIT_REMOTE_SUBS,
    ),
    sca("show", "Show various types of objects", ArgKind::GitRef),
    sca(
        "cherry-pick",
        "Apply changes introduced by existing commits",
        ArgKind::GitRef,
    ),
    sc("cherry", "Find commits not yet applied upstream"),
    sc(
        "bisect",
        "Use binary search to find the commit that introduced a bug",
    ),
    sc("init", "Create an empty Git repository"),
    sc("grep", "Print lines matching a pattern"),
    sc("revert", "Revert some existing commits"),
    sc("rev-parse", "Pick out and massage parameters"),
    sc("check-ignore", "Debug gitignore / exclude file rules"),
    sc(
        "checkout-index",
        "Copy files from the index to the working tree",
    ),
    sca(
        "rm",
        "Remove files from the working tree and index",
        ArgKind::Path,
    ),
    sca(
        "mv",
        "Move or rename a file, directory, or symlink",
        ArgKind::Path,
    ),
    sc("config", "Get and set repository or global options"),
    sc("worktree", "Manage multiple working trees"),
];

// ==========================================================================
// cargo
// ==========================================================================

const CARGO_BUILD: &[OptionSpec] = &[
    o(
        "-r",
        "--release",
        "Build in release mode with optimizations",
    ),
    o("", "--all-features", "Activate all available features"),
    o(
        "",
        "--no-default-features",
        "Do not activate the default feature",
    ),
    oar("-p", "--package", "Build only the given package"),
    o("", "--workspace", "Build all packages in the workspace"),
    oa("", "--bin", "Build only the given binary"),
    oa("", "--example", "Build only the given example"),
    oa("", "--features", "Space/comma separated list of features"),
    oa("", "--target", "Build for the given target triple"),
    oa("-j", "--jobs", "Number of parallel jobs"),
    o("-v", "--verbose", "Use verbose output"),
    o("-q", "--quiet", "Do not print cargo log messages"),
];
const CARGO_SUBS: &[SubcommandSpec] = &[
    sco("build", "Compile the current package", CARGO_BUILD),
    sco(
        "run",
        "Run a binary or example of the local package",
        CARGO_BUILD,
    ),
    sco("test", "Run the tests", CARGO_BUILD),
    sco("check", "Check the current package for errors", CARGO_BUILD),
    sc("clippy", "Check with the Clippy linter"),
    sc("fmt", "Format all source files with rustfmt"),
    sc("doc", "Build the documentation"),
    sca("new", "Create a new cargo package", ArgKind::Dir),
    sca(
        "init",
        "Create a package in an existing directory",
        ArgKind::Dir,
    ),
    sc("add", "Add a dependency to a manifest"),
    sc("remove", "Remove a dependency from a manifest"),
    sc("update", "Update dependencies in Cargo.lock"),
    sc("publish", "Upload a package to the registry"),
    sc("install", "Install a Rust binary"),
    sc("clean", "Remove generated artifacts"),
    sco("bench", "Run the benchmarks", CARGO_BUILD),
    sc("tree", "Display the dependency graph"),
    sc("fix", "Automatically fix lint warnings"),
    sc("search", "Search packages on crates.io"),
];

// ==========================================================================
// node package managers
// ==========================================================================

const NPM_INSTALL: &[OptionSpec] = &[
    o("-g", "--global", "Install the package globally"),
    o("-D", "--save-dev", "Save to devDependencies"),
    o("-S", "--save", "Save to dependencies (default)"),
    o("-E", "--save-exact", "Save the exact version"),
    o("", "--save-peer", "Save to peerDependencies"),
];
const NPM_SUBS: &[SubcommandSpec] = &[
    sco("install", "Install dependencies", NPM_INSTALL),
    sco("i", "Install dependencies (alias)", NPM_INSTALL),
    sca(
        "run",
        "Run a script defined in package.json",
        ArgKind::PkgScript,
    ),
    sca(
        "run-script",
        "Run a script defined in package.json",
        ArgKind::PkgScript,
    ),
    sc("start", "Run the start script"),
    sc("test", "Run the test script"),
    sc("ci", "Clean install from the lockfile"),
    sc("init", "Create a package.json"),
    sc("update", "Update packages"),
    sc("uninstall", "Remove a package"),
    sc("exec", "Run a command from a package"),
    sc("run-many", "Run scripts"),
    sc("publish", "Publish a package"),
    sc("link", "Symlink a package folder"),
    sc("ls", "List installed packages"),
    sc("outdated", "Check for outdated packages"),
    sc("audit", "Run a security audit"),
    sc("version", "Bump a package version"),
];
const YARN_SUBS: &[SubcommandSpec] = &[
    sca("run", "Run a defined package script", ArgKind::PkgScript),
    sc("add", "Add a dependency"),
    sc("remove", "Remove a dependency"),
    sc("install", "Install all dependencies"),
    sc("start", "Run the start script"),
    sc("test", "Run the test script"),
    sc("build", "Run the build script"),
    sc("dev", "Run the dev script"),
    sc("upgrade", "Upgrade dependencies"),
    sc("why", "Show why a package is installed"),
    sc("init", "Create a package.json"),
];
const PNPM_SUBS: &[SubcommandSpec] = &[
    sca("run", "Run a defined package script", ArgKind::PkgScript),
    sc("install", "Install all dependencies"),
    sc("add", "Add a dependency"),
    sc("remove", "Remove a dependency"),
    sc("update", "Update dependencies"),
    sc("start", "Run the start script"),
    sc("test", "Run the test script"),
    sc("exec", "Execute a shell command in scope"),
    sc("dlx", "Fetch a package and run it"),
    sc("why", "Show why a package is installed"),
];

// ==========================================================================
// docker / kubectl / brew / gh / go / rustup / systemctl / apt
// ==========================================================================

const DOCKER_COMPOSE_SUBS: &[SubcommandSpec] = &[
    sc("up", "Create and start containers"),
    sc("down", "Stop and remove containers and networks"),
    sc("build", "Build or rebuild services"),
    sc("logs", "View output from containers"),
    sc("ps", "List containers"),
    sc("exec", "Execute a command in a running container"),
    sc("run", "Run a one-off command"),
    sc("restart", "Restart services"),
    sc("pull", "Pull service images"),
    sc("stop", "Stop services"),
    sc("start", "Start services"),
    sc("config", "Parse, resolve and render the compose file"),
];
const DOCKER_SUBS: &[SubcommandSpec] = &[
    sc("run", "Create and run a new container from an image"),
    sc("build", "Build an image from a Dockerfile"),
    sc("ps", "List containers"),
    sc("images", "List images"),
    sc("pull", "Download an image from a registry"),
    sc("push", "Upload an image to a registry"),
    sc("exec", "Execute a command in a running container"),
    sc("logs", "Fetch the logs of a container"),
    sc("stop", "Stop one or more running containers"),
    sc("start", "Start one or more stopped containers"),
    sc("rm", "Remove one or more containers"),
    sc("rmi", "Remove one or more images"),
    scn(
        "compose",
        "Define and run multi-container applications",
        DOCKER_COMPOSE_SUBS,
    ),
    sc("inspect", "Return low-level information on Docker objects"),
    sc("tag", "Create a tag that refers to an image"),
    sc("login", "Log in to a registry"),
    sc("system", "Manage Docker"),
];

const KUBECTL_ROLLOUT_SUBS: &[SubcommandSpec] = &[
    sc("status", "Show the status of a rollout"),
    sc("history", "View rollout history"),
    sc("undo", "Roll back to a previous revision"),
    sc("restart", "Restart a resource"),
    sc("pause", "Pause a rollout"),
    sc("resume", "Resume a paused rollout"),
];
const KUBECTL_SUBS: &[SubcommandSpec] = &[
    sc("get", "Display one or many resources"),
    sc("describe", "Show details of a specific resource"),
    sc("apply", "Apply a configuration to a resource"),
    sc("delete", "Delete resources"),
    sc("create", "Create a resource"),
    sc("logs", "Print the logs for a container in a pod"),
    sc("exec", "Execute a command in a container"),
    sc("run", "Run a particular image in the cluster"),
    sc("scale", "Set a new size for a deployment or replica set"),
    scn(
        "rollout",
        "Manage the rollout of a resource",
        KUBECTL_ROLLOUT_SUBS,
    ),
    sc("port-forward", "Forward local ports to a pod"),
    sc("expose", "Expose a resource as a new service"),
    sc("edit", "Edit a resource on the server"),
    sc("top", "Display resource usage"),
    sc("config", "Modify kubeconfig files"),
    sc("cluster-info", "Display cluster info"),
    sc("explain", "Get documentation for a resource"),
    sc("version", "Print client and server version"),
];

const BREW_SERVICES_SUBS: &[SubcommandSpec] = &[
    sc("start", "Start a service"),
    sc("stop", "Stop a service"),
    sc("restart", "Restart a service"),
    sc("list", "List all managed services"),
    sc("run", "Run a service without registering it"),
];
const BREW_SUBS: &[SubcommandSpec] = &[
    sc("install", "Install a formula or cask"),
    sc("uninstall", "Uninstall a formula or cask"),
    sc("update", "Fetch the newest version of Homebrew"),
    sc("upgrade", "Upgrade outdated formulae and casks"),
    sc("list", "List installed formulae and casks"),
    sc("search", "Search for formulae and casks"),
    sc("info", "Display information about a formula or cask"),
    sc("tap", "Tap a formula repository"),
    sc("untap", "Untap a formula repository"),
    sc("doctor", "Check your system for potential problems"),
    sc("cleanup", "Remove stale downloads and old versions"),
    scn("services", "Manage background services", BREW_SERVICES_SUBS),
    sc("pin", "Pin a formula to its current version"),
    sc("unpin", "Unpin a formula"),
    sc("outdated", "List outdated formulae and casks"),
];

const GH_PR_SUBS: &[SubcommandSpec] = &[
    sc("create", "Create a pull request"),
    sc("list", "List pull requests"),
    sc("view", "View a pull request"),
    sc("checkout", "Check out a pull request locally"),
    sc("merge", "Merge a pull request"),
    sc("close", "Close a pull request"),
    sc("status", "Show status of relevant pull requests"),
    sc("diff", "View changes in a pull request"),
    sc("review", "Add a review to a pull request"),
    sc("ready", "Mark a pull request as ready for review"),
    sc("edit", "Edit a pull request"),
];
const GH_ISSUE_SUBS: &[SubcommandSpec] = &[
    sc("create", "Create an issue"),
    sc("list", "List issues"),
    sc("view", "View an issue"),
    sc("close", "Close an issue"),
    sc("edit", "Edit an issue"),
    sc("status", "Show status of relevant issues"),
    sc("comment", "Add a comment to an issue"),
];
const GH_REPO_SUBS: &[SubcommandSpec] = &[
    sc("create", "Create a repository"),
    sc("clone", "Clone a repository"),
    sc("view", "View a repository"),
    sc("fork", "Fork a repository"),
    sc("list", "List repositories"),
    sc("delete", "Delete a repository"),
];
const GH_RUN_SUBS: &[SubcommandSpec] = &[
    sc("list", "List recent workflow runs"),
    sc("view", "View a workflow run"),
    sc("watch", "Watch a workflow run until it completes"),
    sc("rerun", "Rerun a workflow run"),
];
const GH_AUTH_SUBS: &[SubcommandSpec] = &[
    sc("login", "Log in to a GitHub host"),
    sc("logout", "Log out of a GitHub host"),
    sc("status", "View authentication status"),
    sc("refresh", "Refresh stored authentication credentials"),
];
const GH_SUBS: &[SubcommandSpec] = &[
    scn("pr", "Manage pull requests", GH_PR_SUBS),
    scn("issue", "Manage issues", GH_ISSUE_SUBS),
    scn("repo", "Manage repositories", GH_REPO_SUBS),
    scn("run", "View details about workflow runs", GH_RUN_SUBS),
    scn("auth", "Authenticate gh and git with GitHub", GH_AUTH_SUBS),
    sc("release", "Manage releases"),
    sc("workflow", "View and run workflows"),
    sc("gist", "Manage gists"),
    sc("api", "Make an authenticated GitHub API request"),
    sc("browse", "Open the repository in the browser"),
    sc("status", "Print information about relevant issues and PRs"),
];

const GO_MOD_SUBS: &[SubcommandSpec] = &[
    sc("init", "Initialize a new module"),
    sc("tidy", "Add missing and remove unused modules"),
    sc("download", "Download modules to the local cache"),
    sc("vendor", "Make a vendored copy of dependencies"),
    sc("verify", "Verify dependencies have expected content"),
    sc("edit", "Edit go.mod from tools or scripts"),
    sc("graph", "Print the module requirement graph"),
    sc("why", "Explain why packages or modules are needed"),
];
const GO_SUBS: &[SubcommandSpec] = &[
    sc("build", "Compile packages and dependencies"),
    sc("run", "Compile and run a Go program"),
    sc("test", "Test packages"),
    sc("get", "Add dependencies and install"),
    sc("install", "Compile and install packages"),
    scn("mod", "Module maintenance", GO_MOD_SUBS),
    sc("fmt", "Reformat package sources with gofmt"),
    sc("vet", "Report likely mistakes in packages"),
    sc("generate", "Generate Go files by processing source"),
    sc("clean", "Remove object files and cached files"),
    sc("list", "List packages or modules"),
    sc("version", "Print Go version"),
    sc("doc", "Show documentation for a package or symbol"),
];

const RUSTUP_TOOLCHAIN_SUBS: &[SubcommandSpec] = &[
    sc("list", "List installed toolchains"),
    sc("install", "Install or update a toolchain"),
    sc("uninstall", "Uninstall a toolchain"),
    sc("link", "Create a custom toolchain"),
];
const RUSTUP_TARGET_SUBS: &[SubcommandSpec] = &[
    sc("list", "List available targets"),
    sc("add", "Add a target to a toolchain"),
    sc("remove", "Remove a target from a toolchain"),
];
const RUSTUP_COMPONENT_SUBS: &[SubcommandSpec] = &[
    sc("list", "List available components"),
    sc("add", "Add a component to a toolchain"),
    sc("remove", "Remove a component from a toolchain"),
];
const RUSTUP_SUBS: &[SubcommandSpec] = &[
    sc("update", "Update Rust toolchains and rustup"),
    sc("default", "Set the default toolchain"),
    scn(
        "toolchain",
        "Modify or query the installed toolchains",
        RUSTUP_TOOLCHAIN_SUBS,
    ),
    scn(
        "target",
        "Modify a toolchain's supported targets",
        RUSTUP_TARGET_SUBS,
    ),
    scn(
        "component",
        "Modify a toolchain's installed components",
        RUSTUP_COMPONENT_SUBS,
    ),
    sc("show", "Show the active and installed toolchains"),
    sc("which", "Display which binary will be run"),
    sc("run", "Run a command with a given toolchain"),
    sc("doc", "Open the documentation for the current toolchain"),
    sc("override", "Modify per-directory toolchain overrides"),
];

const SYSTEMCTL_SUBS: &[SubcommandSpec] = &[
    sc("start", "Start (activate) one or more units"),
    sc("stop", "Stop (deactivate) one or more units"),
    sc("restart", "Restart one or more units"),
    sc("reload", "Reload one or more units"),
    sc("status", "Show runtime status of one or more units"),
    sc("enable", "Enable one or more units"),
    sc("disable", "Disable one or more units"),
    sc("is-active", "Check whether units are active"),
    sc("is-enabled", "Check whether units are enabled"),
    sc("list-units", "List loaded units"),
    sc("list-unit-files", "List installed unit files"),
    sc("daemon-reload", "Reload the systemd manager configuration"),
    sc("mask", "Mask one or more units"),
    sc("unmask", "Unmask one or more units"),
    sc("cat", "Show the unit files and their contents"),
];
const APT_YES: &[OptionSpec] = &[o("-y", "--yes", "Assume yes to all prompts")];
const APT_SUBS: &[SubcommandSpec] = &[
    sco("install", "Install packages", APT_YES),
    sco("remove", "Remove packages", APT_YES),
    sc("update", "Update the list of available packages"),
    sco("upgrade", "Upgrade installed packages", APT_YES),
    sc("search", "Search for packages"),
    sc("show", "Show package details"),
    sc("list", "List packages"),
    sco("purge", "Remove packages and their configuration", APT_YES),
    sco(
        "autoremove",
        "Remove automatically installed packages",
        APT_YES,
    ),
    sco(
        "full-upgrade",
        "Upgrade, removing packages if needed",
        APT_YES,
    ),
];
const PIP_SUBS: &[SubcommandSpec] = &[
    sc("install", "Install packages"),
    sc("uninstall", "Uninstall packages"),
    sc("list", "List installed packages"),
    sc("show", "Show information about packages"),
    sc("freeze", "Output installed packages in requirements format"),
    sc("download", "Download packages"),
    sc("config", "Manage local and global configuration"),
];

// ==========================================================================
// plain commands with just options / an arg generator
// ==========================================================================

const LS_OPTS: &[OptionSpec] = &[
    o("-l", "", "Use a long listing format"),
    o("-a", "--all", "Do not ignore entries starting with ."),
    o("-h", "--human-readable", "Print human-readable sizes"),
    o("-R", "--recursive", "List subdirectories recursively"),
    o("-t", "", "Sort by modification time, newest first"),
    o("-S", "", "Sort by file size, largest first"),
    o("-r", "--reverse", "Reverse the order of the sort"),
    o("-1", "", "List one file per line"),
];
const GREP_OPTS: &[OptionSpec] = &[
    o("-i", "--ignore-case", "Ignore case distinctions"),
    o("-r", "--recursive", "Read all files under each directory"),
    o("-n", "--line-number", "Prefix each line with its number"),
    o("-v", "--invert-match", "Select non-matching lines"),
    o(
        "-E",
        "--extended-regexp",
        "Interpret the pattern as an extended regex",
    ),
    o(
        "-l",
        "--files-with-matches",
        "Print only names of matching files",
    ),
    o("-c", "--count", "Print only a count of matching lines"),
    o("-w", "--word-regexp", "Match only whole words"),
    oa("-A", "--after-context", "Print N lines of trailing context"),
    oa("-B", "--before-context", "Print N lines of leading context"),
    oa("-C", "--context", "Print N lines of output context"),
    o("", "--color", "Highlight matches"),
];
const RG_OPTS: &[OptionSpec] = &[
    o("-i", "--ignore-case", "Case-insensitive search"),
    o(
        "-S",
        "--smart-case",
        "Case-insensitive if the pattern is lowercase",
    ),
    o(
        "-w",
        "--word-regexp",
        "Only show matches surrounded by word boundaries",
    ),
    o("-n", "--line-number", "Show line numbers"),
    o("-v", "--invert-match", "Invert matching"),
    o(
        "-l",
        "--files-with-matches",
        "Print only paths with matches",
    ),
    o("-c", "--count", "Show the number of matching lines"),
    o(
        "-F",
        "--fixed-strings",
        "Treat the pattern as a literal string",
    ),
    o("", "--hidden", "Search hidden files and directories"),
    oar("-e", "--regexp", "A pattern to search for"),
    oar("-t", "--type", "Only search files of the given type"),
    oa("-A", "--after-context", "Show N lines after each match"),
    oa("-B", "--before-context", "Show N lines before each match"),
    oa("-C", "--context", "Show N lines around each match"),
];
const FIND_OPTS: &[OptionSpec] = &[
    oa("", "-name", "Base of file name matches a pattern"),
    oa("", "-iname", "Case-insensitive -name"),
    oa("", "-type", "File is of the given type (f, d, l, ...)"),
    oa("", "-path", "Path name matches a pattern"),
    oa("", "-maxdepth", "Descend at most N directory levels"),
    oa("", "-mindepth", "Do not apply tests at levels below N"),
    oa("", "-mtime", "File data was modified N*24 hours ago"),
    oa("", "-size", "File uses N units of space"),
    o("", "-delete", "Delete found files"),
    o("", "-exec", "Execute a command on found files"),
];
const TAR_OPTS: &[OptionSpec] = &[
    o("-c", "--create", "Create a new archive"),
    o("-x", "--extract", "Extract files from an archive"),
    o("-t", "--list", "List the contents of an archive"),
    o("-z", "--gzip", "Filter the archive through gzip"),
    o("-j", "--bzip2", "Filter the archive through bzip2"),
    o("-v", "--verbose", "List files processed"),
    oa("-f", "--file", "Use the given archive file"),
    oa("-C", "--directory", "Change to the given directory"),
];
const CURL_OPTS: &[OptionSpec] = &[
    oa("-o", "--output", "Write output to a file"),
    o("-O", "--remote-name", "Save with the remote file name"),
    o("-L", "--location", "Follow redirects"),
    o("-s", "--silent", "Silent mode"),
    oa("-X", "--request", "Specify the request method"),
    oar("-H", "--header", "Pass a custom header"),
    oa("-d", "--data", "Send the given data in a POST request"),
    o("-I", "--head", "Fetch the headers only"),
    oa("-u", "--user", "Server user and password"),
    o("-k", "--insecure", "Allow insecure server connections"),
    o("-f", "--fail", "Fail silently on HTTP errors"),
    o("-v", "--verbose", "Make the operation more talkative"),
];
const SSH_OPTS: &[OptionSpec] = &[
    oa("-p", "", "Port to connect to on the remote host"),
    oa("-i", "", "Identity (private key) file"),
    oa("-l", "", "Log in using the given user"),
    oa("-L", "", "Local port forwarding"),
    oa("-R", "", "Remote port forwarding"),
    o("-A", "", "Enable agent forwarding"),
    o("-X", "", "Enable X11 forwarding"),
    o("-t", "", "Force pseudo-terminal allocation"),
    o("-N", "", "Do not execute a remote command"),
    o("-v", "", "Verbose mode"),
];
const MAKE_OPTS: &[OptionSpec] = &[
    oa("-j", "--jobs", "Allow N jobs at once"),
    o("-B", "--always-make", "Unconditionally make all targets"),
    o("-n", "--dry-run", "Print commands without executing"),
    oa("-C", "--directory", "Change to the given directory first"),
    o("-k", "--keep-going", "Keep going when some targets fail"),
    oa("-f", "--file", "Read the given file as the makefile"),
];
/// The full embedded command table.
const SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "git",
        options: GIT_GLOBAL,
        subcommands: GIT_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "cargo",
        options: &[],
        subcommands: CARGO_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "npm",
        options: &[],
        subcommands: NPM_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "yarn",
        options: &[],
        subcommands: YARN_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "pnpm",
        options: &[],
        subcommands: PNPM_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "docker",
        options: &[],
        subcommands: DOCKER_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "kubectl",
        options: &[],
        subcommands: KUBECTL_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "brew",
        options: &[],
        subcommands: BREW_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "gh",
        options: &[],
        subcommands: GH_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "go",
        options: &[],
        subcommands: GO_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "rustup",
        options: &[],
        subcommands: RUSTUP_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "systemctl",
        options: &[],
        subcommands: SYSTEMCTL_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "apt",
        options: &[],
        subcommands: APT_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "apt-get",
        options: &[],
        subcommands: APT_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "pip",
        options: &[],
        subcommands: PIP_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "pip3",
        options: &[],
        subcommands: PIP_SUBS,
        args: ArgKind::None,
    },
    CommandSpec {
        name: "make",
        options: MAKE_OPTS,
        subcommands: &[],
        args: ArgKind::MakeTarget,
    },
    CommandSpec {
        name: "ssh",
        options: SSH_OPTS,
        subcommands: &[],
        args: ArgKind::SshHost,
    },
    CommandSpec {
        name: "scp",
        options: &[],
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "cd",
        options: &[],
        subcommands: &[],
        args: ArgKind::Dir,
    },
    CommandSpec {
        name: "ls",
        options: LS_OPTS,
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "grep",
        options: GREP_OPTS,
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "rg",
        options: RG_OPTS,
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "find",
        options: FIND_OPTS,
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "tar",
        options: TAR_OPTS,
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "curl",
        options: CURL_OPTS,
        subcommands: &[],
        args: ArgKind::None,
    },
    CommandSpec {
        name: "python",
        options: &[],
        subcommands: &[],
        args: ArgKind::Path,
    },
    CommandSpec {
        name: "python3",
        options: &[],
        subcommands: &[],
        args: ArgKind::Path,
    },
];

/// Look up a top-level command spec by exact name.
pub fn lookup(cmd: &str) -> Option<&'static CommandSpec> {
    SPECS.iter().find(|s| s.name == cmd)
}

// ==========================================================================
// resolution
// ==========================================================================

/// The completion context after walking a command's already-typed arguments.
pub struct Resolved {
    /// Options available at the resolved level (command globals + descended
    /// subcommand options).
    pub options: Vec<OptionSpec>,
    /// Subcommands offered for the next token (empty once a positional began).
    pub subcommands: &'static [SubcommandSpec],
    /// Generator for the current positional argument.
    pub args: ArgKind,
    /// Whether the next token may still be a subcommand.
    pub offer_subcommands: bool,
    /// Option-shaped words already present (for dedup of non-repeatable flags).
    pub used_options: Vec<String>,
}

fn find_option<'a>(opts: &'a [OptionSpec], word: &str) -> Option<&'a OptionSpec> {
    let key = word.split('=').next().unwrap_or(word);
    opts.iter()
        .find(|o| (!o.short.is_empty() && o.short == key) || (!o.long.is_empty() && o.long == key))
}

/// Walk `args` (everything after the command name) through the spec tree.
pub fn resolve(cmd: &CommandSpec, args: &[String]) -> Resolved {
    let mut options: Vec<OptionSpec> = cmd.options.to_vec();
    let mut subs: &'static [SubcommandSpec] = cmd.subcommands;
    let mut arg_kind = cmd.args;
    let mut offer_subcommands = !cmd.subcommands.is_empty();
    let mut used_options: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let w = &args[i];
        if w.starts_with('-') && w != "-" && w != "--" {
            used_options.push(w.clone());
            if find_option(&options, w)
                .map(|o| o.takes_arg)
                .unwrap_or(false)
                && !w.contains('=')
            {
                i += 1; // this flag consumes the following value word
            }
            i += 1;
            continue;
        }
        if let Some(s) = subs.iter().find(|s| s.name == *w) {
            // the command's own options go before its subcommand (`git -p
            // log`; after it, `-p` is log's `--patch`); a nested
            // subcommand keeps its parent's
            if std::ptr::eq(subs, cmd.subcommands) {
                options.clear();
            }
            options.extend_from_slice(s.options);
            subs = s.subcommands;
            arg_kind = s.args;
            offer_subcommands = !s.subcommands.is_empty();
            i += 1;
            continue;
        }
        // a positional that is not a subcommand: stop offering subcommands but
        // keep the generator active for any further positionals.
        offer_subcommands = false;
        i += 1;
    }

    Resolved {
        options,
        subcommands: subs,
        args: arg_kind,
        offer_subcommands,
        used_options,
    }
}

// ==========================================================================
// shell tokenizer (simple quoting, pipe/;/&&/|| aware)
// ==========================================================================

/// Byte offset in `prefix` at which the last top-level command begins, i.e.
/// just after the final unquoted `|`, `||`, `&&`, `;` or newline. Returns 0 when there
/// is no separator. The returned index is always a char boundary.
pub fn segment_start(prefix: &str) -> usize {
    let b = prefix.as_bytes();
    let mut i = 0;
    let mut start = 0;
    let mut quote: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                i += 1;
            }
            None => match c {
                b'\'' | b'"' => {
                    quote = Some(c);
                    i += 1;
                }
                b'\\' => i += 2,
                b';' | b'\n' => {
                    i += 1;
                    start = i;
                }
                b'|' => {
                    i += if i + 1 < b.len() && b[i + 1] == b'|' {
                        2
                    } else {
                        1
                    };
                    start = i;
                }
                b'&' => {
                    if i + 1 < b.len() && b[i + 1] == b'&' {
                        i += 2;
                        start = i;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            },
        }
    }
    start.min(prefix.len())
}

/// Split a command segment into words, honoring single/double quotes and
/// backslash escapes. A trailing unterminated word is included.
pub fn tokenize(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut quote: Option<char> = None;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
                has = true;
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    has = true;
                }
                '\\' => {
                    if let Some(n) = chars.next() {
                        cur.push(n);
                        has = true;
                    }
                }
                c if c.is_whitespace() => {
                    if has {
                        words.push(std::mem::take(&mut cur));
                        has = false;
                    }
                }
                _ => {
                    cur.push(c);
                    has = true;
                }
            },
        }
    }
    if has {
        words.push(cur);
    }
    words
}

// ==========================================================================
// generators (filesystem only, bounded, never panic)
// ==========================================================================

const MAX_GEN: usize = 400;

fn cwd_path(cwd: Option<&str>) -> PathBuf {
    PathBuf::from(cwd.unwrap_or("."))
}

/// Walk up from `cwd` to locate the `.git` directory (handles `.git` files
/// that point elsewhere, e.g. worktrees). Never recurses without bound.
fn find_git_dir(cwd: Option<&str>) -> Option<PathBuf> {
    let mut cur: Option<PathBuf> = Some(cwd_path(cwd));
    let mut steps = 0;
    while let Some(dir) = cur {
        steps += 1;
        if steps > 64 {
            return None;
        }
        let g = dir.join(".git");
        if g.is_dir() {
            return Some(g);
        }
        if g.is_file()
            && let Ok(s) = fs::read_to_string(&g)
            && let Some(rest) = s.trim().strip_prefix("gitdir:")
        {
            let p = PathBuf::from(rest.trim());
            return Some(if p.is_absolute() { p } else { dir.join(p) });
        }
        cur = dir.parent().map(Path::to_path_buf);
    }
    None
}

/// Directory holding the shared refs and config: the git dir itself, or, in
/// a linked worktree (`<repo>/.git/worktrees/<n>`), its `commondir`.
fn common_git_dir(gitdir: PathBuf) -> PathBuf {
    match fs::read_to_string(gitdir.join("commondir")) {
        Ok(s) if !s.trim().is_empty() => {
            let p = PathBuf::from(s.trim());
            if p.is_absolute() { p } else { gitdir.join(p) }
        }
        _ => gitdir,
    }
}

/// Values a cached generator list keeps (bounded memory for huge repos or
/// `known_hosts` files).
const MAX_LIST: usize = 100_000;
/// Generator lists kept (per repository, directory or home).
const LIST_CACHE: usize = 32;

/// Which generator a cached list belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum List {
    GitRefs,
    GitRemotes,
    SshHosts,
    MakeTargets,
    PkgScripts,
}

/// Generator values: (value, description).
pub type Values = Vec<(String, String)>;

type ListCache = crate::completion_cache::Lru<(List, PathBuf), Values>;

static LISTS: std::sync::LazyLock<std::sync::Mutex<ListCache>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(ListCache::new(LIST_CACHE)));

/// The cached `kind` list for `key`, read by `load` (the values and every
/// path they came from) when missing or when one of those paths changed.
fn cached_list(
    kind: List,
    key: PathBuf,
    load: impl FnOnce() -> (Values, Vec<PathBuf>),
) -> Arc<Values> {
    crate::completion_cache::cached(&LISTS, &(kind, key), load)
}

/// The `prefix` matches of `all`, at most `MAX_GEN`.
fn with_prefix(all: &[(String, String)], prefix: &str) -> Values {
    all.iter()
        .filter(|(v, _)| v.starts_with(prefix))
        .take(MAX_GEN)
        .cloned()
        .collect()
}

/// Loose refs under `root` (`a/b` names); every directory walked goes to
/// `dirs` (a ref added or removed changes its directory's stamp).
fn collect_refs(
    root: &Path,
    kind: &str,
    seen: &mut HashSet<String>,
    out: &mut Values,
    dirs: &mut Vec<PathBuf>,
) {
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        dirs.push(dir.clone());
        if out.len() >= MAX_LIST {
            return;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push((e.path(), rel));
            } else if seen.insert(rel.clone()) {
                out.push((rel, kind.to_string()));
                if out.len() >= MAX_LIST {
                    return;
                }
            }
        }
    }
}

/// Every git ref reachable from `cwd`, cached per repository: local
/// branches, then remote branches, then tags, each sorted by name. Value =
/// ref name, description = "branch" / "remote" / "tag". Read again when
/// `HEAD`, `packed-refs` or any directory under `refs/` changed.
pub fn git_refs_all(cwd: Option<&str>) -> Arc<Values> {
    let Some(gitdir) = find_git_dir(cwd).map(common_git_dir) else {
        return Arc::default();
    };
    cached_list(List::GitRefs, gitdir.clone(), || {
        let mut out = Vec::new();
        let mut dirs = vec![gitdir.join("HEAD"), gitdir.join("packed-refs")];
        let mut seen = HashSet::new();
        for (sub, kind) in [
            ("refs/heads", "branch"),
            ("refs/remotes", "remote"),
            ("refs/tags", "tag"),
        ] {
            collect_refs(&gitdir.join(sub), kind, &mut seen, &mut out, &mut dirs);
        }
        if let Ok(s) = fs::read_to_string(gitdir.join("packed-refs")) {
            for line in s.lines() {
                if out.len() >= MAX_LIST {
                    break;
                }
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
                    continue;
                }
                let Some((_sha, r)) = line.split_once(' ') else {
                    continue;
                };
                let (name, kind) = if let Some(b) = r.strip_prefix("refs/heads/") {
                    (b, "branch")
                } else if let Some(t) = r.strip_prefix("refs/tags/") {
                    (t, "tag")
                } else if let Some(rm) = r.strip_prefix("refs/remotes/") {
                    (rm, "remote")
                } else {
                    continue;
                };
                if seen.insert(name.to_string()) {
                    out.push((name.to_string(), kind.to_string()));
                }
            }
        }
        let order = |k: &str| match k {
            "branch" => 0,
            "remote" => 1,
            _ => 2,
        };
        out.sort_by(|a, b| order(&a.1).cmp(&order(&b.1)).then_with(|| a.0.cmp(&b.0)));
        (out, dirs)
    })
}

/// The git refs reachable from `cwd` that start with `prefix` (see
/// [`git_refs_all`]), at most `MAX_GEN`.
pub fn git_refs(cwd: Option<&str>, prefix: &str) -> Vec<(String, String)> {
    with_prefix(&git_refs_all(cwd), prefix)
}

/// Configured git remotes (parsed from `.git/config`), cached until the
/// config file changes.
pub fn git_remotes(cwd: Option<&str>) -> Vec<(String, String)> {
    let Some(gitdir) = find_git_dir(cwd).map(common_git_dir) else {
        return Vec::new();
    };
    let config = gitdir.join("config");
    let all = cached_list(List::GitRemotes, gitdir, || {
        let s = fs::read_to_string(&config).unwrap_or_default();
        (parse_git_remotes(&s), vec![config.clone()])
    });
    (*all).clone()
}

fn parse_git_remotes(config: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in config.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("[remote \"")
            && let Some(name) = rest.strip_suffix("\"]")
        {
            out.push((name.to_string(), "remote".to_string()));
        }
    }
    out
}

/// Hosts from `~/.ssh/config` (`Host` entries, wildcards skipped, `Include`d
/// files followed) and plain hostnames from `~/.ssh/known_hosts`, cached
/// until one of those files (or a directory an `Include` glob lists)
/// changes.
pub fn ssh_hosts_all() -> Arc<Values> {
    match std::env::var_os("HOME") {
        Some(home) => ssh_hosts_in(PathBuf::from(home)),
        None => Arc::default(),
    }
}

/// [`ssh_hosts_all`] for the home directory `base`.
fn ssh_hosts_in(base: PathBuf) -> Arc<Values> {
    cached_list(List::SshHosts, base.clone(), || {
        let mut names: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let (hosts, mut deps) = crate::ssh::config_hosts_and_sources(&base.join(".ssh/config"));
        for h in hosts {
            if seen.insert(h.clone()) {
                names.push(h);
            }
        }
        let known = base.join(".ssh/known_hosts");
        if let Ok(s) = fs::read_to_string(&known) {
            for h in parse_known_hosts(&s) {
                if names.len() >= MAX_LIST {
                    break;
                }
                if seen.insert(h.clone()) {
                    names.push(h);
                }
            }
        }
        deps.push(known);
        let values = names
            .into_iter()
            .map(|n| (n, "ssh host".to_string()))
            .collect();
        (values, deps)
    })
}

/// The ssh hosts starting with `prefix` (see [`ssh_hosts_all`]), at most
/// `MAX_GEN`.
pub fn ssh_hosts(prefix: &str) -> Vec<(String, String)> {
    with_prefix(&ssh_hosts_all(), prefix)
}

fn parse_known_hosts(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let l = line.trim();
        // comment, hashed entry, or a marker line (`@cert-authority`,
        // `@revoked`): its patterns name keys' scopes, not hosts
        if l.is_empty() || l.starts_with('#') || l.starts_with('|') || l.starts_with('@') {
            continue;
        }
        let Some(field) = l.split_whitespace().next() else {
            continue;
        };
        for host in field.split(',') {
            // strip [host]:port bracket form → host
            let host = host
                .strip_prefix('[')
                .map(|h| h.split(']').next().unwrap_or(h))
                .unwrap_or(host);
            // patterns (`*`, `?`) and negations (`!host`) are no host
            if host.is_empty() || host.contains(['*', '?']) || host.starts_with('!') {
                continue;
            }
            out.push(host.to_string());
        }
    }
    out
}

/// The makefile names read, first found wins.
const MAKEFILES: [&str; 3] = ["Makefile", "makefile", "GNUmakefile"];

/// Every target declared in a Makefile in `cwd`, cached until one of the
/// makefile names there changes (created, edited or removed).
pub fn make_targets_all(cwd: Option<&str>) -> Arc<Values> {
    let base = cwd_path(cwd);
    cached_list(List::MakeTargets, base.clone(), || {
        let files: Vec<PathBuf> = MAKEFILES.iter().map(|n| base.join(n)).collect();
        let targets = files
            .iter()
            .find_map(|f| fs::read_to_string(f).ok())
            .map(|s| parse_make_targets_n(&s, "", MAX_LIST))
            .unwrap_or_default();
        (targets, files)
    })
}

/// The Makefile targets in `cwd` starting with `prefix`, at most `MAX_GEN`.
pub fn make_targets(cwd: Option<&str>, prefix: &str) -> Vec<(String, String)> {
    with_prefix(&make_targets_all(cwd), prefix)
}

#[cfg(test)]
fn parse_make_targets(content: &str, prefix: &str) -> Vec<(String, String)> {
    parse_make_targets_n(content, prefix, MAX_GEN)
}

fn parse_make_targets_n(content: &str, prefix: &str, cap: usize) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in content.lines() {
        // recipe lines start with a tab; skip them and blank/comment lines
        if line.starts_with(char::is_whitespace) || line.trim_start().starts_with('#') {
            continue;
        }
        let Some(idx) = line.find(':') else {
            continue;
        };
        // variable assignment (":=") is not a target
        if line.as_bytes().get(idx + 1) == Some(&b'=') {
            continue;
        }
        let lhs = &line[..idx];
        if lhs.contains('=') {
            continue;
        }
        for t in lhs.split_whitespace() {
            if t.starts_with('.') || t.contains('%') || t.contains('$') {
                continue; // special targets, pattern rules, variable refs
            }
            if t.starts_with(prefix) && seen.insert(t.to_string()) {
                out.push((t.to_string(), "target".to_string()));
            }
        }
        if out.len() >= cap {
            break;
        }
    }
    out
}

/// Every script name (with its command as the description) from the
/// package.json in `cwd`, cached until that file changes. Uses a minimal
/// hand-rolled scan — no serde_json dependency.
pub fn package_scripts_all(cwd: Option<&str>) -> Arc<Values> {
    let file = cwd_path(cwd).join("package.json");
    cached_list(List::PkgScripts, file.clone(), || {
        let scripts = fs::read_to_string(&file)
            .map(|s| parse_package_scripts_n(&s, "", MAX_LIST))
            .unwrap_or_default();
        (scripts, vec![file.clone()])
    })
}

/// The package.json scripts in `cwd` starting with `prefix`, at most
/// `MAX_GEN`.
pub fn package_scripts(cwd: Option<&str>, prefix: &str) -> Vec<(String, String)> {
    with_prefix(&package_scripts_all(cwd), prefix)
}

/// Read the JSON string whose opening quote is at `b[start]`; returns the
/// decoded text and the index just past the closing quote.
fn read_json_string(b: &[u8], start: usize) -> (String, usize) {
    fn hex4(b: &[u8], at: usize) -> Option<u32> {
        let h = std::str::from_utf8(b.get(at..at + 4)?).ok()?;
        u32::from_str_radix(h, 16).ok()
    }
    fn push_char(val: &mut Vec<u8>, c: char) {
        let mut buf = [0u8; 4];
        val.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    let mut j = start + 1;
    let mut val: Vec<u8> = Vec::new();
    while j < b.len() {
        match b[j] {
            b'\\' if j + 1 < b.len() => match b[j + 1] {
                b'n' => {
                    val.push(b'\n');
                    j += 2;
                }
                b't' => {
                    val.push(b'\t');
                    j += 2;
                }
                b'r' => {
                    val.push(b'\r');
                    j += 2;
                }
                b'b' => {
                    val.push(0x08);
                    j += 2;
                }
                b'f' => {
                    val.push(0x0c);
                    j += 2;
                }
                b'u' => {
                    let Some(mut cp) = hex4(b, j + 2) else {
                        val.push(b'u');
                        j += 2;
                        continue;
                    };
                    j += 6;
                    if (0xD800..0xDC00).contains(&cp)
                        && b.get(j) == Some(&b'\\')
                        && b.get(j + 1) == Some(&b'u')
                        && let Some(lo) = hex4(b, j + 2)
                        && (0xDC00..0xE000).contains(&lo)
                    {
                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                        j += 6;
                    }
                    push_char(&mut val, char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                // `\"`, `\\`, `\/` and anything else: the byte itself
                c => {
                    val.push(c);
                    j += 2;
                }
            },
            b'"' => break,
            c => {
                val.push(c);
                j += 1;
            }
        }
    }
    (String::from_utf8_lossy(&val).into_owned(), j + 1)
}

#[cfg(test)]
fn parse_package_scripts(json: &str, prefix: &str) -> Vec<(String, String)> {
    parse_package_scripts_n(json, prefix, MAX_GEN)
}

fn parse_package_scripts_n(json: &str, prefix: &str, cap: usize) -> Vec<(String, String)> {
    let Some(pos) = json.find("\"scripts\"") else {
        return Vec::new();
    };
    let rest = &json[pos..];
    let Some(brace) = rest.find('{') else {
        return Vec::new();
    };
    let body = &rest[brace + 1..];
    let b = body.as_bytes();

    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    let mut depth = 1i32;
    let mut expect_key = true;
    let mut last_key: Option<usize> = None;
    let mut i = 0;
    while i < b.len() && depth >= 1 {
        match b[i] {
            b'"' => {
                let (val, next) = read_json_string(b, i);
                i = next;
                if depth == 1 {
                    if expect_key {
                        if val.starts_with(prefix) && seen.insert(val.clone()) {
                            out.push((val, String::new()));
                            last_key = Some(out.len() - 1);
                        }
                        expect_key = false;
                    } else if let Some(k) = last_key.take() {
                        // 80 characters: never cut inside a multi-byte one
                        out[k].1 = val.chars().take(80).collect();
                    }
                }
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                i += 1;
            }
            b',' => {
                if depth == 1 {
                    expect_key = true;
                }
                i += 1;
            }
            _ => i += 1,
        }
        if out.len() >= cap {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lookup_and_flat_subcommands() {
        let git = lookup("git").expect("git spec");
        let r = resolve(git, &[]);
        assert!(r.offer_subcommands);
        assert!(r.subcommands.iter().any(|s| s.name == "commit"));
        assert!(lookup("definitely-not-a-command").is_none());
    }

    #[test]
    fn resolve_descends_into_subcommand() {
        let git = lookup("git").unwrap();
        let r = resolve(git, &words(&["checkout"]));
        assert!(!r.offer_subcommands, "checkout has no subcommands");
        assert_eq!(r.args, ArgKind::GitRef);
        // git's own options belong before the subcommand
        assert!(r.options.iter().any(|o| o.long == "--detach"));
        assert!(!r.options.iter().any(|o| o.long == "--paginate"));
        assert!(
            resolve(git, &[])
                .options
                .iter()
                .any(|o| o.long == "--paginate")
        );
        // commit-level option is merged in
        let r = resolve(git, &words(&["commit"]));
        assert!(r.options.iter().any(|o| o.long == "--message"));
    }

    #[test]
    fn resolve_nested_subcommands() {
        let docker = lookup("docker").unwrap();
        let r = resolve(docker, &words(&["compose"]));
        assert!(r.offer_subcommands, "compose exposes nested subcommands");
        assert!(r.subcommands.iter().any(|s| s.name == "up"));
        assert!(r.subcommands.iter().any(|s| s.name == "down"));
        // one level deeper: `docker compose up` is terminal
        let r = resolve(docker, &words(&["compose", "up"]));
        assert!(!r.offer_subcommands);
    }

    #[test]
    fn resolve_marks_used_options_and_skips_values() {
        let git = lookup("git").unwrap();
        // -m consumes "msg"; the trailing bare word is a positional, not a sub
        let r = resolve(git, &words(&["commit", "-m", "msg"]));
        assert!(r.used_options.iter().any(|u| u == "-m"));
        assert!(!r.offer_subcommands);
        assert_eq!(r.args, ArgKind::Path);
    }

    #[test]
    fn tokenize_respects_quotes() {
        assert_eq!(
            tokenize("git commit -m \"a b c\""),
            words(&["git", "commit", "-m", "a b c"])
        );
        assert_eq!(tokenize("echo 'one two'"), words(&["echo", "one two"]));
        assert_eq!(tokenize("a\\ b"), words(&["a b"]));
    }

    #[test]
    fn segment_start_splits_on_operators() {
        assert_eq!(segment_start("git status"), 0);
        let p = "cat x | git ";
        assert_eq!(&p[segment_start(p)..], " git ");
        let p = "a && b ";
        assert_eq!(&p[segment_start(p)..], " b ");
        let p = "a; b";
        assert_eq!(&p[segment_start(p)..], " b");
        // a pipe inside quotes is NOT a separator
        let p = "echo \"a | b\" ";
        assert_eq!(segment_start(p), 0);
    }

    #[test]
    fn git_refs_reads_loose_and_packed() {
        let dir = unique_tmp("refs");
        let heads = dir.join(".git/refs/heads/feature");
        fs::create_dir_all(&heads).unwrap();
        fs::write(dir.join(".git/refs/heads/main"), "0\n").unwrap();
        fs::write(heads.join("x"), "0\n").unwrap();
        fs::write(
            dir.join(".git/packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n\
             aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111 refs/tags/v1.0\n\
             bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222 refs/heads/release\n",
        )
        .unwrap();

        let refs = git_refs(Some(dir.to_str().unwrap()), "");
        let names: Vec<&str> = refs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"main"), "loose branch: {names:?}");
        assert!(
            names.contains(&"feature/x"),
            "nested loose branch: {names:?}"
        );
        assert!(names.contains(&"v1.0"), "packed tag: {names:?}");
        assert!(names.contains(&"release"), "packed branch: {names:?}");
        assert!(refs.iter().any(|(n, k)| n == "v1.0" && k == "tag"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_remotes_parse() {
        let cfg = "[core]\n\trepositoryformatversion = 0\n\
                   [remote \"origin\"]\n\turl = git@example.com:me/repo.git\n\
                   [remote \"upstream\"]\n\turl = https://example.com/up.git\n";
        let r = parse_git_remotes(cfg);
        let names: Vec<&str> = r.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["origin", "upstream"]);
    }

    #[test]
    fn ssh_config_parsing_skips_wildcards() {
        let cfg = "Host prod bastion\n  HostName 10.0.0.1\n\
                   Host *.internal\n  User admin\n\
                   Host web-*\n\
                   host DEV\n";
        let path = std::env::temp_dir().join(format!("rift-spec-ssh-{}", std::process::id()));
        std::fs::write(&path, cfg).unwrap();
        let hosts = crate::ssh::config_hosts(&path);
        std::fs::remove_file(&path).ok();
        assert!(hosts.contains(&"prod".to_string()));
        assert!(hosts.contains(&"bastion".to_string()));
        assert!(
            hosts.contains(&"DEV".to_string()),
            "Host key is case-insensitive"
        );
        assert!(!hosts.iter().any(|h| h.contains('*')));
    }

    #[test]
    fn known_hosts_parsing() {
        let kh = "github.com,140.82.112.3 ssh-rsa AAAA...\n\
                  [example.com]:2222 ssh-ed25519 BBBB...\n\
                  |1|hashedstuff= ssh-rsa CCCC...\n\
                  @cert-authority ca.example.org ssh-ed25519 DDDD...\n\
                  @revoked revoked.example.org ssh-rsa EEEE...\n\
                  h?.example.org,!bad.example.org ssh-rsa FFFF...\n";
        let hosts = parse_known_hosts(kh);
        for not_host in [
            "@cert-authority",
            "@revoked",
            "ca.example.org",
            "revoked.example.org",
            "h?.example.org",
            "!bad.example.org",
        ] {
            assert!(
                !hosts.contains(&not_host.to_string()),
                "{not_host}: {hosts:?}"
            );
        }
        assert!(hosts.contains(&"github.com".to_string()));
        assert!(
            hosts.contains(&"example.com".to_string()),
            "bracket/port stripped"
        );
        assert!(
            !hosts.iter().any(|h| h.contains('|')),
            "hashed entries skipped"
        );
    }

    #[test]
    fn makefile_target_parsing() {
        let mk = "CC := gcc\n\
                  .PHONY: all clean\n\
                  all: build test\n\
                  \tgcc -o out\n\
                  build:\n\
                  \tcargo build\n\
                  clean test:\n\
                  \trm -rf out\n\
                  %.o: %.c\n";
        let t = parse_make_targets(mk, "");
        let names: Vec<&str> = t.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"all"));
        assert!(names.contains(&"build"));
        assert!(names.contains(&"clean"));
        assert!(names.contains(&"test"));
        assert!(
            !names.contains(&"CC"),
            "variable assignment is not a target"
        );
        assert!(!names.iter().any(|n| n.starts_with('.')), ".PHONY skipped");
        assert!(
            !names.iter().any(|n| n.contains('%')),
            "pattern rule skipped"
        );
    }

    #[test]
    fn package_json_scripts_parsing() {
        let pkg = "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\",\n\
                   \"scripts\": {\n    \"build\": \"tsc -p .\",\n\
                   \"test\": \"jest --ci\",\n    \"dev\": \"vite\"\n  },\n\
                   \"dependencies\": { \"left-pad\": \"^1.0.0\" }\n}\n";
        let s = parse_package_scripts(pkg, "");
        let names: Vec<&str> = s.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["build", "test", "dev"]);
        // description carries the script's command, and we stop at the closing
        // brace (dependencies keys are not scripts)
        assert_eq!(s[0].1, "tsc -p .");
        assert!(!names.contains(&"left-pad"));
    }

    #[test]
    fn package_scripts_decode_non_ascii_and_escapes_without_panicking() {
        // > 80 bytes, every char multi-byte: a byte-wise truncate(80) would
        // land inside one
        let long = "é".repeat(60);
        let pkg = format!(
            "{{\"scripts\": {{\"a\": \"{long}\", \"b\": \"caf\\u00e9 \\ud83d\\ude80 \\\"q\\\" \\n\", \
             \"c\": \"日本語\"}}}}"
        );
        let s = parse_package_scripts(&pkg, "");
        assert_eq!(s[0].1, "é".repeat(60), "60 chars < 80 chars: kept whole");
        assert_eq!(s[1].1, "café 🚀 \"q\" \n");
        assert_eq!(s[2].1, "日本語");
        let long = "é".repeat(100);
        let pkg = format!("{{\"scripts\": {{\"a\": \"{long}\"}}}}");
        assert_eq!(parse_package_scripts(&pkg, "")[0].1, "é".repeat(80));
    }

    #[test]
    fn generators_filter_by_prefix_before_the_cap() {
        let dir = unique_tmp("cap");
        let heads = dir.join(".git/refs/heads");
        fs::create_dir_all(&heads).unwrap();
        for n in 0..(MAX_GEN + 50) {
            fs::write(heads.join(format!("a{n:04}")), "0\n").unwrap();
        }
        fs::write(heads.join("zzz-wanted"), "0\n").unwrap();
        let cwd = dir.to_str();
        assert!(git_refs(cwd, "").len() <= MAX_GEN);
        let r = git_refs(cwd, "zzz");
        assert_eq!(r, vec![("zzz-wanted".to_string(), "branch".to_string())]);
        let mk: String = (0..MAX_GEN + 50)
            .map(|n| format!("t{n}:\n"))
            .collect::<String>()
            + "wanted:\n";
        assert_eq!(parse_make_targets(&mk, "wan").len(), 1);
        let mut pkg = String::from("{\"scripts\": {");
        for n in 0..MAX_GEN + 50 {
            pkg.push_str(&format!("\"s{n}\": \"x\", "));
        }
        pkg.push_str("\"wanted\": \"y\"}}");
        assert_eq!(parse_package_scripts(&pkg, "wan").len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn linked_worktree_reads_refs_and_remotes_from_the_common_dir() {
        let main = unique_tmp("wt-main");
        let common = main.join(".git");
        fs::create_dir_all(common.join("refs/heads")).unwrap();
        fs::create_dir_all(common.join("refs/remotes/origin")).unwrap();
        fs::write(common.join("refs/heads/main"), "0\n").unwrap();
        fs::write(common.join("refs/remotes/origin/dev"), "0\n").unwrap();
        fs::write(common.join("config"), "[remote \"origin\"]\n\turl = x\n").unwrap();
        let wtgit = common.join("worktrees/feature");
        fs::create_dir_all(&wtgit).unwrap();
        fs::write(wtgit.join("commondir"), "../..\n").unwrap();
        let wt = unique_tmp("wt-linked");
        fs::write(wt.join(".git"), format!("gitdir: {}\n", wtgit.display())).unwrap();
        let cwd = wt.to_str();
        let names: Vec<String> = git_refs(cwd, "").into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"main".to_string()), "{names:?}");
        assert!(names.contains(&"origin/dev".to_string()), "{names:?}");
        assert_eq!(git_remotes(cwd), vec![("origin".into(), "remote".into())]);
        let _ = fs::remove_dir_all(&main);
        let _ = fs::remove_dir_all(&wt);
    }

    use crate::completion_cache::tests::age;

    fn names(v: &[(String, String)]) -> Vec<&str> {
        v.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn git_refs_cache_follows_new_refs_and_packed_refs() {
        let dir = unique_tmp("refcache");
        let git = dir.join(".git");
        let heads = git.join("refs/heads");
        fs::create_dir_all(heads.join("team")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(heads.join("main"), "0\n").unwrap();
        fs::write(heads.join("team/a"), "0\n").unwrap();
        // only the paths a change touched get a new mtime, as in real life
        for p in [&git, &git.join("refs"), &heads, &heads.join("team")] {
            age(p, 100);
        }
        age(&git.join("HEAD"), 100);
        let cwd = dir.to_str();
        assert_eq!(names(&git_refs_all(cwd)), ["main", "team/a"]);
        // a ref added in a nested directory: only that directory changes
        fs::write(heads.join("team/b"), "0\n").unwrap();
        age(&heads.join("team"), 90);
        assert_eq!(names(&git_refs_all(cwd)), ["main", "team/a", "team/b"]);
        // a new nested directory
        fs::create_dir_all(heads.join("x/y")).unwrap();
        fs::write(heads.join("x/y/z"), "0\n").unwrap();
        for p in [&heads, &heads.join("x"), &heads.join("x/y")] {
            age(p, 80);
        }
        assert!(names(&git_refs_all(cwd)).contains(&"x/y/z"));
        // packed-refs written (git pack-refs, fetch of many tags)
        fs::write(
            git.join("packed-refs"),
            "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111 refs/tags/v9\n",
        )
        .unwrap();
        age(&git.join("packed-refs"), 70);
        assert_eq!(git_refs_all(cwd).last().unwrap().0, "v9");
        // a ref removed
        fs::remove_file(heads.join("team/a")).unwrap();
        age(&heads.join("team"), 60);
        assert!(!names(&git_refs_all(cwd)).contains(&"team/a"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn makefile_and_package_json_caches_follow_edits() {
        let dir = unique_tmp("filecache");
        let cwd = dir.to_str();
        let mk = dir.join("Makefile");
        fs::write(&mk, "build:\n").unwrap();
        age(&mk, 100);
        assert_eq!(names(&make_targets_all(cwd)), ["build"]);
        fs::write(&mk, "build:\ntest:\n").unwrap();
        age(&mk, 90);
        assert_eq!(names(&make_targets_all(cwd)), ["build", "test"]);
        let pkg = dir.join("package.json");
        assert!(package_scripts_all(cwd).is_empty());
        fs::write(&pkg, r#"{"scripts": {"dev": "vite"}}"#).unwrap();
        age(&pkg, 100);
        assert_eq!(names(&package_scripts_all(cwd)), ["dev"]);
        fs::write(&pkg, r#"{"scripts": {"dev": "vite", "lint": "x"}}"#).unwrap();
        age(&pkg, 90);
        assert_eq!(names(&package_scripts_all(cwd)), ["dev", "lint"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_remotes_cache_follows_the_config() {
        let dir = unique_tmp("remotecache");
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        let config = git.join("config");
        fs::write(&config, "[remote \"origin\"]\n").unwrap();
        age(&config, 100);
        let cwd = dir.to_str();
        assert_eq!(names(&git_remotes(cwd)), ["origin"]);
        fs::write(&config, "[remote \"origin\"]\n[remote \"fork\"]\n").unwrap();
        age(&config, 90);
        assert_eq!(names(&git_remotes(cwd)), ["origin", "fork"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ssh_hosts_cache_follows_config_includes_and_known_hosts() {
        let home = unique_tmp("sshcache");
        let ssh = home.join(".ssh");
        let conf_d = home.join("conf.d");
        fs::create_dir_all(&ssh).unwrap();
        fs::create_dir_all(&conf_d).unwrap();
        let config = ssh.join("config");
        fs::write(
            &config,
            format!("Host alpha\nInclude {}/*.conf\n", conf_d.display()),
        )
        .unwrap();
        age(&config, 100);
        age(&conf_d, 100);
        assert_eq!(names(&ssh_hosts_in(home.clone())), ["alpha"]);
        // a new file matching the Include glob
        let inc = conf_d.join("work.conf");
        fs::write(&inc, "Host beta\n").unwrap();
        age(&inc, 90);
        age(&conf_d, 90);
        assert_eq!(names(&ssh_hosts_in(home.clone())), ["alpha", "beta"]);
        // the included file edited
        fs::write(&inc, "Host beta gamma\n").unwrap();
        age(&inc, 80);
        assert_eq!(
            names(&ssh_hosts_in(home.clone())),
            ["alpha", "beta", "gamma"]
        );
        // known_hosts created
        let known = ssh.join("known_hosts");
        fs::write(&known, "delta.example ssh-ed25519 AAAA\n").unwrap();
        age(&known, 70);
        assert!(names(&ssh_hosts_in(home.clone())).contains(&"delta.example"));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn newline_starts_a_new_segment() {
        let p = "ls\ngit ";
        assert_eq!(&p[segment_start(p)..], "git ");
        // a newline inside quotes is not a separator
        let p = "echo 'a\nb' ";
        assert_eq!(segment_start(p), 0);
    }

    // temp dir helper: unique path under the system temp dir
    fn unique_tmp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p =
            std::env::temp_dir().join(format!("rift-comp-{tag}-{}-{t}-{n}", std::process::id()));
        let _ = fs::create_dir_all(&p);
        p
    }
}
