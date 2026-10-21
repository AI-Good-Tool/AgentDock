#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! AgentDock entry: desktop app by default; CLI flags for headless and
//! scheduled operation (carried over from the original tools).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let arg = |flag: &str, n: usize| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + n).cloned())
    };
    if has("--server") || has("--claim") || has("--dump-state") {
        let runtime = tokio::runtime::Runtime::new().expect("async runtime");
        let result: Result<(), String> = runtime.block_on(async {
            let hub = agentdock::open_hub()?;
            if has("--claim") {
                let r = agentdock::tasks::claim_once(&hub).await;
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
                return Ok(());
            }
            if has("--dump-state") {
                let models = hub.upstream_models();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "dataDir": hub.dir,
                        "settings": hub.settings(),
                        "accounts": hub.pool_accounts().len(),
                        "upstreamModels": models.len(),
                        "routes": agentdock::catalog::build(&models, &hub.settings()).iter().map(|r| &r.alias).collect::<Vec<_>>(),
                    }))
                    .unwrap()
                );
                return Ok(());
            }
            tokio::spawn(agentdock::tasks::spawn_all(hub.clone()));
            let port = agentdock::server::start(hub.clone()).await?;
            println!("AgentDock gateway http://127.0.0.1:{port}/v1 （本地 API Key 见 {}/api-key）", hub.dir.display());
            tokio::signal::ctrl_c().await.map_err(|_| "signal failure")?;
            agentdock::server::stop(&hub);
            Ok(())
        });
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    // Qoder debugging commands from Qoder Switch.
    if has("--sessions") || has("--cstats") || has("--tstats") || has("--copy") || has("--switch") {
        let dir = agentdock::paths::data_dir();
        let _ = agentdock::paths::prepare(&dir);
        qoder::qcore::set_data_dir(dir.join("qoder"));
        if let Some(ed) = arg("--sessions", 1) {
            let r = qoder::qsession::list_sessions(&ed, "", 20).unwrap_or_else(|e| serde_json::json!({"error": e}));
            println!("{}", serde_json::to_string_pretty(&r).unwrap());
        } else if has("--cstats") {
            println!("{}", serde_json::to_string(&qoder::qsession::credit_stats(None)).unwrap());
        } else if let Some(ed) = arg("--tstats", 1) {
            match qoder::qstats::token_stats(&ed, None) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
                Err(e) => println!("{{\"error\":\"{e}\"}}"),
            }
        } else if let (Some(ed), Some(sid)) = (arg("--copy", 1), arg("--copy", 2)) {
            match qoder::qsession::copy_session(&ed, &sid) {
                Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap()),
                Err(e) => eprintln!("复制失败：{e}"),
            }
        } else if let Some(id) = arg("--switch", 1) {
            let mut steps = vec![];
            let result = qoder::qcore::switch_account(&id, &mut steps);
            for s in &steps {
                eprintln!("[step] {s}");
            }
            match result {
                Ok(msg) => println!("OK: {msg}"),
                Err(e) => {
                    eprintln!("切换失败：{e}");
                    std::process::exit(1);
                }
            }
        }
        return;
    }
    agentdock::run_desktop();
}
