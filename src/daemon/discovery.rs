use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::plugin_config_dir;
use crate::daemon::worker;
use crate::herdr::client;
use crate::log::{log, log_debug, log_error};
use crate::tools::{self, ToolDetector};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Discovery loop: polls every 2 seconds for new panes. Finds newly
/// appearing panes and hands them off to a per-pane worker thread that uses
/// `wait_output` instead of being re-scanned here. Tries every registered
/// `ToolDetector` against each newly seen pane's process list; first match
/// wins. Never returns under normal operation.
pub fn run() -> ! {
    let config_dir = plugin_config_dir();
    let detectors: Vec<Arc<dyn ToolDetector>> = tools::all_detectors(&config_dir)
        .into_iter()
        .map(Arc::from)
        .collect();

    let tracked: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let (done_tx, done_rx): (Sender<String>, Receiver<String>) = channel();
    log(&format!(
        "discovery loop starting, pid={}",
        std::process::id()
    ));
    let mut tick_count: u64 = 0;

    loop {
        // Drain workers that finished (pane's process died, or pane vanished).
        while let Ok(pane_id) = done_rx.try_recv() {
            log(&format!(
                "pane {pane_id}: worker exited, eligible for re-detection"
            ));
            tracked.lock().unwrap().remove(&pane_id);
        }

        tick_count += 1;
        let pane_ids = match client::list_panes() {
            Ok(ids) => ids,
            Err(err) => {
                log_error(
                    &format!(
                        "tick {tick_count}: failed to list panes (is 'pane list' the right subcommand?)"
                    ),
                    &err,
                );
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        };

        let tracked_snapshot = tracked.lock().unwrap().clone();
        log_debug(&format!(
            "tick {tick_count}: {} pane(s): [{}], tracking {}",
            pane_ids.len(),
            pane_ids.join(", "),
            tracked_snapshot.len()
        ));

        for pane_id in &pane_ids {
            if tracked_snapshot.contains(pane_id) {
                continue;
            }

            let procs = match client::get_pane_process_info(pane_id) {
                Ok(p) => p,
                Err(err) => {
                    log_error(
                        &format!(
                            "pane {pane_id}: get_pane_process_info failed (check 'pane process-info')"
                        ),
                        &err,
                    );
                    continue;
                }
            };

            let Some((detector, m)) = detectors
                .iter()
                .find_map(|d| d.confirm(&procs).map(|m| (Arc::clone(d), m)))
            else {
                continue;
            };

            let detail_str = m
                .details
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", ");
            log(&format!(
                "pane {pane_id}: CONFIRMED {}, pid={}, {detail_str}, cwd={}",
                detector.source(),
                m.pid,
                m.cwd.as_deref().unwrap_or("unknown")
            ));

            {
                let mut guard = tracked.lock().unwrap();
                guard.insert(pane_id.clone());
            }

            // Shared seq for this pair of initial reports — using the
            // process‑wide counter ensures strict monotonicity per `--source`.
            let agent_seq = client::next_seq();
            let metadata_seq = client::next_seq();
            if let Err(err) = client::report_agent_state(
                pane_id,
                detector.source(),
                detector.agent_name(),
                "working",
                Some(detector.starting_message()),
                agent_seq,
            ) {
                log_error(
                    &format!("pane {pane_id}: failed to report initial agent state"),
                    &err,
                );
            }
            if let Err(err) = client::report_metadata(
                pane_id,
                detector.source(),
                &client::MetadataUpdate {
                    display_agent: Some(detector.display_agent()),
                    tokens: vec![],
                    state_labels: detector.state_labels().to_vec(),
                },
                metadata_seq,
            ) {
                log_error(
                    &format!("pane {pane_id}: failed to report initial metadata"),
                    &err,
                );
            }

            let worker_pane_id = pane_id.clone();
            let worker_done_tx = done_tx.clone();
            let worker_detector = Arc::clone(&detector);
            thread::spawn(move || {
                worker::run(worker_pane_id, m.pid, worker_detector, worker_done_tx)
            });
        }

        thread::sleep(POLL_INTERVAL);
    }
}
