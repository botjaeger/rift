//! The history pane's Results scope: asking the search worker
//! (`crate::results`) and taking its answers.

use super::*;

impl App {
    /// The pane's run identity: what `RIFT_PANE_ID` names (its service
    /// session when durable).
    pub(super) fn pane_run_id(p: &PaneState) -> String {
        match p.session.service_id() {
            Some(sid) => rift_core::uri::PaneRef::Service(sid).id(),
            None => rift_core::uri::PaneRef::Local {
                run: rift_core::uri::run_token().into(),
                pane: p.session.id,
            }
            .id(),
        }
    }

    /// Ask for the Results scope's search when its query has none yet:
    /// every window's open panes, and the saved transcripts while
    /// `save_transcripts` is on.
    pub(super) fn search_results_if_wanted(&mut self) {
        if !self.win.history.wants_search() {
            return;
        }
        let panes = self
            .all_panes()
            .map(|p| crate::results::PaneFeed {
                pane: Self::pane_run_id(p),
                transcript: p.transcript_id.clone(),
                restored: self.config.save_transcripts
                    && !self.transcripts_cleared.contains(&p.session.id),
                term: p.session.term.clone(),
                blocks: p.session.blocks.clone(),
                scrolled: p.session.scrolled.clone(),
            })
            .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let local = crate::views::prompt_chips::LocalTime::now();
        let today_start = now
            .saturating_sub(u64::from(local.hour) * 3600 + u64::from(local.minute) * 60 + now % 60);
        let asker = self.win_id();
        let hp = &mut self.win.history;
        hp.results_seq += 1;
        hp.results_query = Some(hp.query.clone());
        hp.searching = true;
        hp.saved = self.config.save_transcripts;
        let job = crate::results::Job {
            asker,
            seq: hp.results_seq,
            query: hp.query.clone(),
            panes,
            saved: hp.saved,
            now,
            today_start,
        };
        self.results_worker.submit(job, &self.proxy);
    }

    /// The worker's answer: shown when it is for the current query.
    pub(super) fn on_results_found(
        &mut self,
        seq: u64,
        outcome: rift_core::result_search::Outcome,
    ) {
        if self.win.history.take_results(seq, outcome)
            && let Some(w) = &self.win.window
        {
            w.request_redraw();
        }
    }

    /// `dump`: the Results scope's state and the selected run.
    pub(super) fn results_dump(&self) -> String {
        let hp = &self.win.history;
        let sel = hp.current_found();
        format!(
            "history_results={} searching={} truncated={} saved={} results_query={:?} found_selected={:?} found_context={:?} found_when={:?} found_exit={:?} found_preview={:?} found_heading={:?} found_hit={:?} found_ids={:?}",
            hp.found.len(),
            hp.searching,
            hp.truncated,
            hp.saved,
            hp.results_query,
            sel.map(|f| f.command.as_str()),
            sel.map(crate::views::history::found_context),
            sel.map(|f| f.finished.is_some()),
            sel.and_then(|f| f.exit),
            sel.map(|f| f.preview.join("\n")),
            sel.map(crate::views::history::preview_heading),
            sel.and_then(|f| f
                .hit
                .as_ref()
                .map(|(i, r)| f.preview[*i].get(r.clone()).unwrap_or(""))),
            hp.found
                .iter()
                .map(|f| match &f.id {
                    rift_core::result_search::RunId::Live { pane, line } =>
                        format!("{pane}@{line}"),
                    rift_core::result_search::RunId::Saved { transcript, index } => {
                        format!("saved:{transcript}#{index}")
                    }
                })
                .collect::<Vec<_>>(),
        )
    }
}
