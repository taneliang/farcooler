//! The landing transport: a trusted-local bare repository, or a network remote reached with the runner
//! user's own git and ssh setup (ov-1026).
use super::*;

/// A push refspec: the candidate commit onto the target ref.
pub(crate) fn push_target(candidate: &str, target: &str) -> String {
    format!("{candidate}:{target}")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LandingFact {
    binding: Binding,
    identity: CodingLanding,
    candidate: String,
    log_digest: String,
}
impl Exec {
    fn remote_fact(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.landing.json", digest(&binding.effect)))
    }
    pub(super) fn remote_log(&self, binding: &Binding) -> PathBuf {
        self.root.join("receipts").join(format!("{}.push.output", digest(&binding.effect)))
    }
    /// The git program and the whole environment a push or fetch runs with: nothing inherited,
    /// the repository's guard pins, and no global or system configuration.
    pub(super) async fn git_environment(&self) -> Result<(String, std::collections::BTreeMap<String, String>)> {
        self.git_environment_for(None).await
    }
    /// As `git_environment`, for a call that reaches `remote`: an `https://` or `ssh://` remote is allowed
    /// that one transport, and the credentials it signs in with. Every other transport stays refused.
    pub(super) async fn git_environment_for(&self, remote: Option<&str>) -> Result<(String, std::collections::BTreeMap<String, String>)> {
        let git = crate::git_launch::git_launch().map_err(|_| Error::Refused("git unavailable"))?;
        let mut command = std::process::Command::new(&git.program);
        command.env_clear();
        let mut pins = crate::git::guard_pins(&self.repository()).await.map_err(|_| Error::Refused("landing guard pins unavailable"))?;
        pins.push(("protocol.file.allow".into(), "always".into()));
        // A repository with an alternates file runs this through a shell on a fetch into it; the shared repository's
        // config is an agent's to write. Pinned to a program that lists nothing (ov-1026).
        pins.push(("core.alternateRefsCommand".into(), "/usr/bin/true".into()));
        let mut extra = std::collections::BTreeMap::new();
        if let (Some(remote), Some(pr)) = (remote, self.coding.as_ref().and_then(|c| c.pull_request.as_ref())) {
            let (more, env) = super::coding_pr::transport(remote, &pr.gh.argv[0], &pr.gh.environment, std::env::var("SSH_AUTH_SOCK").ok());
            pins.extend(more.into_iter().map(|(k, v)| (k.into(), v.into())));
            extra = env;
        }
        crate::git_guard::apply(&mut command, &pins);
        command.env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_CONFIG_GLOBAL", "/dev/null");
        let mut environment: std::collections::BTreeMap<String, String> = command.get_envs().filter_map(|(k,v)| v.map(|v| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned())))
            .collect();
        environment.extend(extra);
        Ok((git.program.to_string_lossy().into_owned(), environment))
    }
    /// The daemon's own bare repository that every call to a network landing remote runs in (ov-1026). Agents
    /// write the shared repository's config, where keys such as `remote.<url>.pushurl` or
    /// `core.alternateRefsCommand` would steer a fetch or a push, or run a program next to the runner's
    /// credentials. This one is made with no template under the private runtime root and is never handed to an
    /// agent, so its config is empty. Objects reach it, and come back, only by a local transfer that carries
    /// no credentials.
    pub(super) async fn landing_repository(&self) -> Result<PathBuf> {
        let path = self.root.join("landing.git");
        if !path.join("HEAD").is_file() {
            let (program, environment) = self.git_environment().await?;
            let output = tokio::process::Command::new(program).current_dir(&self.root).env_clear().envs(&environment)
                .args(["init", "--bare", "--quiet", "--template="]).arg(&path).stdin(std::process::Stdio::null()).output().await?;
            if !output.status.success() { return Err(Error::Refused("the private landing repository could not be made")); }
        }
        if path.symlink_metadata()?.file_type().is_symlink() || path.canonicalize()? != path {
            return Err(Error::Refused("the private landing repository is an alias"));
        }
        Ok(path)
    }
    /// The git program and environment for a call to the network landing remote `url`, run in the private
    /// landing repository. The one transport the URL names is allowed, and git signs in with the runner user's
    /// own setup (their home, their system and global git config, their ssh config and agent), exactly as it
    /// would in their terminal. The engine adds no credential of its own, so none is written to a receipt.
    pub(super) async fn landing_environment(&self, url: &str) -> Result<(String, std::collections::BTreeMap<String, String>)> {
        let private = self.landing_repository().await?;
        let git = crate::git_launch::git_launch().map_err(|_| Error::Refused("git unavailable"))?;
        let mut command = std::process::Command::new(&git.program);
        command.env_clear();
        let mut pins = crate::git::guard_pins(&private).await.map_err(|_| Error::Refused("landing guard pins unavailable"))?;
        let scheme = if url.starts_with("https://") { "https" } else { "ssh" };
        pins.push((format!("protocol.{scheme}.allow").into(), "always".into()));
        crate::git_guard::apply(&mut command, &pins);
        let mut environment: std::collections::BTreeMap<String, String> = command.get_envs()
            .filter_map(|(k, v)| v.map(|v| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))).collect();
        environment.insert("GIT_DIR".into(), private.to_string_lossy().into_owned());
        environment.insert("HOME".into(), self.user_home()?.to_string_lossy().into_owned());
        environment.entry("PATH".into()).or_insert_with(|| crate::landing_templates::GATE_PATH.into());
        if let Ok(socket) = std::env::var("SSH_AUTH_SOCK") { environment.insert("SSH_AUTH_SOCK".into(), socket); }
        let program = git.program.to_string_lossy().into_owned();
        let output = tokio::process::Command::new(&program).current_dir(&private).env_clear().envs(&environment)
            .args(["config", "--list", "--show-scope", "--includes"]).stdin(std::process::Stdio::null()).output().await?;
        if !output.status.success() { return Err(Error::Refused("the landing config could not be read")); }
        steering(&String::from_utf8_lossy(&output.stdout))?;
        Ok((program, environment))
    }
    /// Run git against the network landing remote `url`, in the private landing repository. Its exit, `None`
    /// when it ran out of time or could not start.
    async fn remote_git(&self, url: &str, args: &[String], log: &Path, limit: Duration, cancel: tokio::sync::watch::Receiver<bool>)
        -> Result<(Vec<String>, std::collections::BTreeMap<String, String>, Option<i32>)>
    {
        let (program, environment) = self.landing_environment(url).await?;
        let argv: Vec<String> = std::iter::once(program).chain(args.iter().cloned()).collect();
        let _ = std::fs::remove_file(log);
        durable(log, b"")?;
        let exit = crate::effect_runner::configured(&self.landing_repository().await?, &argv, &environment, log, limit, cancel).await
            .ok().flatten();
        Ok((argv, environment, exit))
    }
    /// The environment of a local transfer between the shared repository and the private landing one: no
    /// credentials and no user config, as `git_environment`, and nothing that runs after the transfer (gc,
    /// maintenance, a commit graph). A serving side may hand out a commit by its hash.
    pub(super) async fn transfer_environment(&self) -> Result<(String, std::collections::BTreeMap<String, String>)> {
        let (program, mut environment) = self.git_environment().await?;
        let count: usize = environment.get("GIT_CONFIG_COUNT").and_then(|c| c.parse().ok()).ok_or(Error::Refused("unpinned transfer"))?;
        let extra = [("gc.auto", "0"), ("maintenance.auto", "false"), ("fetch.writeCommitGraph", "false"),
            ("uploadpack.allowAnySHA1InWant", "true"), ("transfer.bundleURI", "false"), ("fetch.bundleURI", "")];
        for (i, (key, value)) in extra.iter().enumerate() {
            environment.insert(format!("GIT_CONFIG_KEY_{}", count + i), (*key).into());
            environment.insert(format!("GIT_CONFIG_VALUE_{}", count + i), (*value).into());
        }
        environment.insert("GIT_CONFIG_COUNT".into(), (count + extra.len()).to_string());
        Ok((program, environment))
    }
    /// Bring `head`, already in the private landing repository, into the shared repository's objects, for the fold
    /// to build on. Not a fetch: a fetch into the shared repository runs a ref transaction even when it writes no
    /// ref, and an agent may race a `reference-transaction` hook name into that config past the guard pins. A pack
    /// made in the private repository is indexed there instead, which updates no ref and so fires no hook. It
    /// leaves out what the shared repository is known to hold: its own landing ref, and what was staged from it or
    /// imported before. True when the commit arrived.
    pub(super) async fn import(&self, head: &str, environment: &std::collections::BTreeMap<String, String>, program: &str) -> Result<bool> {
        if !super::coding::sha(head) { return Ok(false); }
        let local = adapters::git(&self.repository(), &["rev-parse", "--verify", "--quiet", self.landing_ref()]).await.ok()
            .filter(|l| super::coding::sha(l));
        if let Some(local) = local { let _ = self.stage(&local, "refs/farcooler/shared/local").await; }
        let known = self.landing_git(&["for-each-ref", "--format=%(objectname)", "refs/farcooler/push", "refs/farcooler/check", "refs/farcooler/shared"])
            .await?.unwrap_or_default();
        let revisions: String = std::iter::once(head.to_string()).chain(known.lines().filter(|l| super::coding::sha(l)).map(|l| format!("^{l}")))
            .map(|l| l + "\n").collect();
        let pack = self.landing_bytes(&["pack-objects", "--revs", "--stdout", "--quiet"], revisions.as_bytes()).await?
            .ok_or(Error::Refused("the landing head could not be packed"))?;
        let mut index = tokio::process::Command::new(program).current_dir(self.repository()).env_clear().envs(environment)
            .args(["index-pack", "--stdin"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).spawn()?;
        if let Some(mut input) = index.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = input.write_all(&pack).await;
        }
        if !index.wait().await?.success() || adapters::git(&self.repository(), &["cat-file", "-e", &format!("{head}^{{commit}}")]).await.is_err() {
            return Ok(false);
        }
        let _ = self.landing_git(&["update-ref", &format!("refs/farcooler/shared/{}", &head[..16]), head]).await;
        Ok(true)
    }
    /// `landing_git` with `input` on stdin, answering stdout's bytes.
    async fn landing_bytes(&self, args: &[&str], input: &[u8]) -> Result<Option<Vec<u8>>> {
        let private = self.landing_repository().await?;
        let (program, mut environment) = self.transfer_environment().await?;
        environment.insert("GIT_DIR".into(), private.to_string_lossy().into_owned());
        environment.insert("GIT_NO_REPLACE_OBJECTS".into(), "1".into());
        let mut child = tokio::process::Command::new(program).current_dir(&private).env_clear().envs(&environment).args(args)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()?;
        let mut stdin = child.stdin.take().ok_or(Error::Refused("no stdin"))?;
        let bytes = input.to_vec();
        let feed = tokio::spawn(async move { use tokio::io::AsyncWriteExt; let _ = stdin.write_all(&bytes).await; });
        let output = child.wait_with_output().await?;
        let _ = feed.await;
        Ok(output.status.success().then_some(output.stdout))
    }
    /// Git in the private landing repository alone, with no credentials and no replace objects: what it answers is
    /// not the shared repository's to change. `None` when git refused.
    async fn landing_git(&self, args: &[&str]) -> Result<Option<String>> {
        let private = self.landing_repository().await?;
        let (program, mut environment) = self.transfer_environment().await?;
        environment.insert("GIT_DIR".into(), private.to_string_lossy().into_owned());
        environment.insert("GIT_NO_REPLACE_OBJECTS".into(), "1".into());
        let output = tokio::process::Command::new(program).current_dir(&private).env_clear().envs(&environment)
            .args(args).stdin(std::process::Stdio::null()).output().await?;
        Ok(output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
    }
    /// Put `candidate` and its history in the private landing repository, so a push from there can send it. What
    /// arrived is read there: a transfer the shared config redirected stages nothing.
    pub(super) async fn stage_candidate(&self, binding: &Binding, candidate: &str) -> Result<String> {
        let reference = format!("refs/farcooler/push/{}", &digest(&binding.effect)[4..20]);
        self.stage(candidate, &reference).await?;
        Ok(reference)
    }
    /// The fetch runs in the private repository, whose config is the engine's; the shared one only serves objects,
    /// which runs no hook.
    pub(super) async fn stage(&self, candidate: &str, reference: &str) -> Result<()> {
        if !super::coding::sha(candidate) { return Err(Error::Refused("missing landing candidate")); }
        let shared = self.repository().to_string_lossy().into_owned();
        let staged = self.landing_git(&["fetch", "--quiet", "--no-tags", "--no-recurse-submodules", "--no-write-fetch-head",
            "--upload-pack=git-upload-pack", &shared, &format!("+{candidate}:{reference}")]).await?.is_some();
        if !staged || self.landing_git(&["rev-parse", "--verify", "--quiet", reference]).await?.as_deref() != Some(candidate) {
            return Err(Error::Refused("the candidate could not be staged for the landing remote"));
        }
        Ok(())
    }
    /// Whether `candidate` builds on `base`, read in the private landing repository. The shared repository's
    /// replace refs, grafts and branches are an agent's to write, and there one `git replace --graft` makes any
    /// commit look like a descendant (ov-1026 review). Either commit missing here is brought over from the shared
    /// repository by its hash first, which carries its real parents.
    pub(super) async fn builds_on(&self, base: &str, candidate: &str) -> Result<bool> {
        for commit in [base, candidate] {
            if !super::coding::sha(commit) { return Ok(false); }
            if self.landing_git(&["cat-file", "-e", &format!("{commit}^{{commit}}")]).await?.is_none() {
                self.stage(commit, &format!("refs/farcooler/check/{}", &commit[..16])).await?;
            }
        }
        Ok(self.landing_git(&["merge-base", "--is-ancestor", base, candidate]).await?.is_some())
    }
    /// Push `candidate` onto the network target from the private landing repository. A plain refspec, never a
    /// force or a lease: the remote itself refuses anything that isn't a fast-forward of what it holds.
    pub(super) async fn push_remote(&self, url: &str, identity: &CodingLanding, candidate: &str,
        log: &Path, cancel: tokio::sync::watch::Receiver<bool>) -> Result<(Vec<String>, std::collections::BTreeMap<String, String>, Option<i32>)>
    {
        self.remote_git(url, &["push".into(), "--porcelain".into(), "--no-follow-tags".into(), "--recurse-submodules=no".into(),
            "--signed=false".into(), url.into(), push_target(candidate, &identity.target)], log, Duration::from_secs(120), cancel).await
    }
    /// The runner user's home, where git and ssh find their own configuration: the daemon's `HOME`, else the
    /// account's home directory.
    fn user_home(&self) -> Result<PathBuf> {
        #[cfg(test)]
        if let Some(home) = &self.user_home { return Ok(home.clone()); }
        if let Some(home) = std::env::var_os("HOME").filter(|h| Path::new(h).is_absolute()) { return Ok(home.into()); }
        // SAFETY: getpwuid returns a pointer into static storage, read at once and copied.
        let entry = unsafe { libc::getpwuid(libc::geteuid()) };
        if entry.is_null() { return Err(Error::Refused("the runner user has no home")); }
        let dir = unsafe { std::ffi::CStr::from_ptr((*entry).pw_dir) };
        Ok(PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(dir.to_bytes())))
    }
    /// The commit at the landing target, asked of wherever it lives. `None` when a network remote didn't answer
    /// or has no such branch: never read as an answer about the target.
    pub(super) async fn target_head(&self, identity: &CodingLanding, log: &Path) -> Result<Option<String>> {
        let Some(url) = super::coding::landing_url(identity) else {
            return Ok(Some(adapters::git(&identity.repository, &["rev-parse", &identity.target]).await?));
        };
        let (_hold, cancel) = tokio::sync::watch::channel(false);
        let (_, _, exit) = self.remote_git(url, &["ls-remote".into(), "--refs".into(), url.into(), identity.target.clone()],
            log, Duration::from_secs(60), cancel).await?;
        if exit != Some(0) { return Ok(None); }
        // Notices on stderr share the log: only a line that is exactly `<sha> TAB <ref>` is an answer.
        Ok(String::from_utf8_lossy(&self.bytes(log)?).lines().find_map(|l| l.split_once('\t')
            .filter(|(sha, name)| *name == identity.target && super::coding::sha(sha)).map(|(sha, _)| sha.to_string())))
    }
    /// Whether `candidate` is on the landing target. A network remote's head is fetched and read here. A remote
    /// that cannot be read is an error, never an answer of "no".
    pub(super) async fn on_target(&self, identity: &CodingLanding, candidate: &str, log: &Path) -> Result<bool> {
        let Some(url) = super::coding::landing_url(identity) else {
            return super::coding_verbs::on_remote(&identity.repository, &identity.target, candidate).await;
        };
        let head = self.fetch_private(identity, url, log).await?.ok_or(Error::Refused("the landing remote cannot be read"))?;
        if head == candidate { return Ok(true); }
        // Read in the private repository, where the shared one's replace refs and rewrites don't reach. The candidate
        // is there if this runner pushed it; if not, it is staged first, and one it can't stage was not pushed from here.
        if self.landing_git(&["cat-file", "-e", &format!("{candidate}^{{commit}}")]).await?.is_none() {
            self.stage(candidate, &format!("refs/farcooler/check/{}", candidate.get(..16).unwrap_or(candidate))).await?;
        }
        Ok(self.landing_git(&["merge-base", "--is-ancestor", candidate, &head]).await?.is_some())
    }
    /// The private ref a landing target's head is fetched into.
    pub(super) fn base_ref(identity: &CodingLanding) -> String {
        format!("refs/farcooler/base/{}", &digest(&(&identity.repository, &identity.target))[4..20])
    }
    /// Fetch the target's head of the network remote `url` into the private landing repository, bring it into the
    /// shared one under the private base ref, and read it. `None` when the fetch failed.
    pub(super) async fn fetch_target(&self, identity: &CodingLanding, url: &str, log: &Path) -> Result<Option<String>> {
        let Some(head) = self.fetch_private(identity, url, log).await? else { return Ok(None) };
        // The shared repository gets the commit for the fold to build on, by its hash, and no ref.
        let (program, environment) = self.transfer_environment().await?;
        Ok(self.import(&head, &environment, &program).await?.then_some(head))
    }
    /// Fetch the target's head into the private landing repository only, and read it there.
    async fn fetch_private(&self, identity: &CodingLanding, url: &str, log: &Path) -> Result<Option<String>> {
        let local = Self::base_ref(identity);
        let (_hold, cancel) = tokio::sync::watch::channel(false);
        let (_, _, exit) = self.remote_git(url, &["fetch".into(), "--quiet".into(), "--no-tags".into(), "--no-recurse-submodules".into(),
            "--no-write-fetch-head".into(), url.into(), format!("+{}:{local}", identity.target)], log, Duration::from_secs(120), cancel).await?;
        if exit != Some(0) { return Ok(None); }
        Ok(self.landing_git(&["rev-parse", "--verify", "--quiet", &local]).await?.filter(|h| super::coding::sha(h)))
    }
    pub(super) async fn land_remote(&self, binding: &Binding, cancelled: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let Some(identity) = self.coding.as_ref().and_then(|c| c.landing.as_ref()) else { return Ok(()) };
        self.coding.as_ref().unwrap().validate()?;
        let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing landing candidate"))?;
        let url = super::coding::landing_url(identity);
        let read_log = |tag: &str| self.root.join("receipts").join(format!("{}.target-{tag}.output", digest(&binding.effect)));
        if self.target_head(identity, &read_log("before")).await? != Some(binding.base.clone()) {
            return Err(Error::Refused("remote landing base moved"));
        }
        if url.is_none() && adapters::git(&identity.repository, &["rev-parse", "--is-bare-repository"]).await? != "true" {
            return Err(Error::Refused("landing transport is not bare"));
        }
        if !self.builds_on(&binding.base, candidate).await? {
            return Err(Error::Refused("the landing candidate does not build on the target"));
        }
        // Disable global/system configuration and refuse URL rewriting/includes
        // in the selected local repository. No named-remote or provider lookup.
        let config = self.git_recorded(binding, &self.repository(), &["config", "--list"]).await?;
        if config.lines().any(|l| l.starts_with("url.") || l.starts_with("include.") || l.starts_with("includeif.")) {
            return Err(Error::Refused("local landing config redirects transport"));
        }
        let log = self.remote_log(binding);
        let (argv, environment, exit) = if let Some(url) = url {
            self.stage_candidate(binding, candidate).await?;
            self.push_remote(url, identity, candidate, &log, cancelled).await?
        } else {
            let (program, environment) = self.git_environment().await?;
            let target = push_target(candidate, &identity.target);
            let argv = vec![program, "push".into(), "--porcelain".into(),
                // Local push policy must not expand this one-ref operation to tags,
                // submodule remotes or signing machinery outside its pinned identity.
                "--no-follow-tags".into(), "--recurse-submodules=no".into(), "--signed=false".into(),
                identity.repository.to_string_lossy().into_owned(), target];
            durable(&log, b"")?;
            let exit = crate::effect_runner::configured(&self.repository(), &argv, &environment, &log,
                Duration::from_secs(60), cancelled).await?;
            (argv, environment, exit)
        };
        durable(&self.root.join("receipts").join(format!("{}.push-process.json", digest(&binding.effect))),
            &serde_json::to_vec(&serde_json::json!({"binding": binding, "identity": identity, "argv": argv,
                "environment": environment, "exit": exit, "log_digest": digest(&self.bytes(&log)?)}))?)?;
        if exit != Some(0) { return Err(Error::Refused("local landing push failed; inspect receipt")); }
        if self.target_head(identity, &read_log("after")).await?.as_deref() != Some(candidate) {
            return Err(Error::Refused("remote accepted SHA unknown"));
        }
        durable(&self.remote_fact(binding), &serde_json::to_vec(&LandingFact { binding: binding.clone(),
            identity: identity.clone(), candidate: candidate.into(), log_digest: digest(&self.bytes(&log)?) })?)
    }
    pub(super) async fn verify_remote_landing(&self, binding: &Binding) -> Result<()> {
        let Some(identity) = self.coding.as_ref().and_then(|c| c.landing.as_ref()) else { return Ok(()) };
        self.coding.as_ref().unwrap().validate()?;
        let candidate = binding.effect.candidate.as_deref().ok_or(Error::Refused("missing remote candidate"))?;
        // CI's own binding differs. It verifies current identity/SHA while Land
        // alone consumes the exact original push settlement. No push on recovery.
        if matches!(binding.effect.kind, EffectKind::Execute(StageKind::Land)) {
            let fact: LandingFact = serde_json::from_slice(&self.bytes(&self.remote_fact(binding))?)?;
            if fact.binding != *binding || fact.identity != *identity || fact.candidate != candidate
                || fact.log_digest != digest(&self.bytes(&self.remote_log(binding))?) {
                return Err(Error::Refused("foreign local landing settlement"));
            }
        }
        // Later landings by other items may sit on top; the candidate only has to be in history.
        let log = self.root.join("receipts").join(format!("{}.verify.fetch.output", digest(&binding.effect)));
        if !super::coding::sha(candidate) || !self.on_target(identity, candidate, &log).await.unwrap_or(false) {
            return Err(Error::Refused("remote landing SHA changed"));
        }
        Ok(())
    }
}

/// The refusal a `git config --list --show-scope` listing of the private landing repository earns, if any. Its own
/// config holds only what `git init` wrote; anything more was put there by someone else. The runner user's system
/// and global config is theirs, except that nothing may send a call for the landing URL somewhere else: a URL
/// rewrite, or a remote named by a URL.
pub(super) fn steering(listing: &str) -> Result<()> {
    const MADE_BY_INIT: [&str; 6] = ["core.repositoryformatversion", "core.filemode", "core.bare", "core.ignorecase",
        "core.precomposeunicode", "core.logallrefupdates"];
    for line in listing.lines() {
        let Some((scope, entry)) = line.split_once('\t') else { continue };
        let key = entry.split('=').next().unwrap_or("").to_ascii_lowercase();
        if key.starts_with("url.") || (key.starts_with("remote.") && key.contains("://")) {
            return Err(Error::Refused("git config sends the landing URL somewhere else; a landing remote is reached exactly as written"));
        }
        if matches!(scope, "local" | "worktree") && !MADE_BY_INIT.contains(&key.as_str()) {
            return Err(Error::Refused("the private landing repository's config was changed"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{coding_tests::{configured, admitted}, tests::{enqueue, finish}};
    pub(super) async fn remote() -> (tempfile::TempDir, Runtime) {
        let (dir, mut runtime) = configured(false, "pass").await;
        let bare = dir.path().join("private-remote.git"); std::fs::create_dir(&bare).unwrap();
        adapters::git(&bare, &["init", "--bare", "--quiet"]).await.unwrap();
        let repo = runtime.repository();
        // Initial private remote population is test setup, never an external push.
        let status = tokio::process::Command::new("/usr/bin/git").current_dir(&repo)
            .args(["-c", "core.hooksPath=/dev/null", "push", bare.to_str().unwrap(), "HEAD:refs/heads/main"])
            .status().await.unwrap(); assert!(status.success());
        let root = runtime.root.clone();
        let config = runtime.coding.as_mut().unwrap();
        config.landing = Some(CodingLanding { job: "landing-928".into(), repository: bare.canonicalize().unwrap(),
            target: "refs/heads/main".into(), config_digest: digest(&std::fs::read(bare.join("config")).unwrap()) });
        std::fs::write(root.join("coding.json"), serde_json::to_vec(config).unwrap()).unwrap();
        drop(runtime);
        (dir, Runtime::open(&root).unwrap())
    }
    #[tokio::test]
    async fn workflow_coding_retained_private_remote_exact_sha_and_repeated_bases() {
        let (_dir, mut runtime) = remote().await;
        let definition = runtime.configured_definition().unwrap();
        for item in ["one", "two"] {
            enqueue(&mut runtime, "w", item, definition.clone());
            let state = finish(&mut runtime, "w").await;
            assert_eq!(state.items[item].phase, Phase::Done);
            let identity = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
            assert_eq!(adapters::git(&identity.repository, &["rev-parse", &identity.target]).await.unwrap(),
                state.items[item].candidate.clone().unwrap());
        }
    }
    #[tokio::test]
    async fn workflow_coding_retained_remote_crash_receipt_and_changed_base_refusal() {
        for crash in [true, false] {
            let (_dir, mut runtime) = remote().await;
            let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
            for time in 2..6 { runtime.pump(time).await.unwrap(); }
            let binding = admitted(&runtime).await;
            let identity = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
            let candidate = binding.effect.candidate.as_ref().unwrap();
            if !crash {
                // Visible exact candidate alone is not this operation's push receipt.
                let status = tokio::process::Command::new("/usr/bin/git").current_dir(runtime.repository())
                    .args(["push", identity.repository.to_str().unwrap(), &push_target(candidate, &identity.target)])
                    .status().await.unwrap(); assert!(status.success());
            }
            let (_hold, cancelled) = tokio::sync::watch::channel(false);
            let result = runtime.execute_coding("w", &binding, cancelled).await;
            assert_eq!(result.is_ok(), crash);
            let root = runtime.root.clone(); drop(runtime);
            let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
            if crash { assert_eq!(finish(&mut runtime, "w").await.items["one"].phase, Phase::Done); }
            else { assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain); }
        }
    }
    #[tokio::test]
    async fn workflow_coding_remote_missing_settlement_after_acceptance_never_pushes_again() {
        let (_dir, mut runtime) = remote().await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..6 { runtime.pump(time).await.unwrap(); }
        let binding = admitted(&runtime).await;
        let (_hold, cancelled) = tokio::sync::watch::channel(false);
        runtime.execute_coding("w", &binding, cancelled).await.unwrap();
        std::fs::remove_file(runtime.remote_fact(&binding)).unwrap();
        let before = std::fs::read(runtime.remote_log(&binding)).unwrap();
        let root = runtime.root.clone(); drop(runtime);
        let mut runtime = Runtime::open(&root).unwrap(); runtime.pump(200).await.unwrap();
        assert_eq!(runtime.state("w").unwrap().unwrap().items["one"].phase, Phase::Uncertain);
        assert_eq!(runtime.pump(201).await.unwrap(), 0);
        assert_eq!(std::fs::read(runtime.remote_log(&binding)).unwrap(), before);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use super::tests::remote;
    use super::super::{coding_tests::admitted, tests::enqueue};
    #[tokio::test]
    async fn workflow_coding_remote_rejected_update_records_exit_without_acceptance() {
        let (_dir, mut runtime) = remote().await;
        let definition = runtime.configured_definition().unwrap(); enqueue(&mut runtime, "w", "one", definition);
        for time in 2..6 { runtime.pump(time).await.unwrap(); }
        let binding = admitted(&runtime).await;
        let identity = runtime.coding.as_ref().unwrap().landing.as_ref().unwrap();
        let lock = identity.repository.join("refs/heads/main.lock"); std::fs::write(&lock, b"private test lock").unwrap();
        let (_hold, cancelled) = tokio::sync::watch::channel(false);
        assert!(runtime.execute_coding("w", &binding, cancelled).await.is_err());
        let fact: serde_json::Value = serde_json::from_slice(&std::fs::read(runtime.root.join("receipts")
            .join(format!("{}.push-process.json", digest(&binding.effect)))).unwrap()).unwrap();
        assert_ne!(fact["exit"], 0);
        assert_eq!(adapters::git(&identity.repository, &["rev-parse", &identity.target]).await.unwrap(), binding.base);
        assert!(!runtime.remote_fact(&binding).exists());
        std::fs::remove_file(lock).unwrap();
        assert!(runtime.reconcile_coding(&binding).await.is_err(), "recovery must not retry the rejected push");
    }
}

#[cfg(test)]
#[path = "coding_security_tests.rs"]
mod security_tests;
