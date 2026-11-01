//! The Replacements editor inside the session actor (REP-8..REP-12): raw
//! row text while typing, a 500 ms debounced write, flushes on disappear,
//! window close, and quit, and no write when nothing changed.

use std::time::Duration;

use onda_domain::replacements::{
    ReplacementRule, RuleId, parse_triggers, rules_to_persist, trigger_text,
};
use uuid::Uuid;

use super::{Actor, Msg, Task, UiMsg};
use crate::seams::SettingChange;
use crate::views::{ReplacementRowView, ReplacementsPage, replacements_page};

/// How long typing must pause before the rules are written (REP-12).
pub(crate) const REPLACEMENTS_DEBOUNCE: Duration = Duration::from_millis(500);

/// One editor row, exactly as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    id: String,
    triggers: String,
    replacement: String,
}

/// Replacements editor state.
#[derive(Default)]
pub(super) struct ReplacementsEditor {
    rows: Vec<Row>,
    revision: u64,
    // what was last loaded or written, in persisted form
    saved: Vec<ReplacementRule>,
    dirty: bool,
    debounce: Option<Task>,
    pub(super) generation: u64,
}

impl ReplacementsEditor {
    fn rules(&self) -> Vec<ReplacementRule> {
        self.rows
            .iter()
            .map(|row| ReplacementRule {
                id: RuleId(row.id.clone()),
                triggers: parse_triggers(&row.triggers),
                replacement: row.replacement.clone(),
            })
            .collect()
    }
}

impl Actor {
    // flush pending edits before reloading on appear (REP-12)
    pub(super) fn reload_replacements(&mut self) {
        self.flush_replacements();
        self.refresh_settings();
        let rules = self.settings.replacement_rules.clone();
        let editor = &mut self.ui.replacements;
        editor.rows = rules
            .iter()
            .map(|rule| Row {
                id: rule.id.0.clone(),
                triggers: trigger_text(&rule.triggers),
                replacement: rule.replacement.clone(),
            })
            .collect();
        editor.saved = rules;
        editor.revision += 1;
    }

    pub(super) fn add_rule(&mut self) {
        let editor = &mut self.ui.replacements;
        editor.rows.push(Row {
            id: Uuid::new_v4().to_string(),
            triggers: String::new(),
            replacement: String::new(),
        });
        editor.revision += 1;
        self.schedule_replacements_write();
    }

    pub(super) fn edit_rule(
        &mut self,
        id: &str,
        triggers: Option<String>,
        replacement: Option<String>,
    ) {
        let Some(row) = self
            .ui
            .replacements
            .rows
            .iter_mut()
            .find(|row| row.id == id)
        else {
            return;
        };
        if let Some(triggers) = triggers {
            row.triggers = triggers;
        }
        if let Some(replacement) = replacement {
            row.replacement = replacement;
        }
        self.schedule_replacements_write();
    }

    pub(super) fn delete_rule(&mut self, id: &str) {
        let editor = &mut self.ui.replacements;
        let before = editor.rows.len();
        editor.rows.retain(|row| row.id != id);
        if editor.rows.len() != before {
            editor.revision += 1;
            self.schedule_replacements_write();
        }
    }

    fn schedule_replacements_write(&mut self) {
        let editor = &mut self.ui.replacements;
        editor.dirty = true;
        editor.generation += 1;
        let generation = editor.generation;
        let timer = self.timer(
            REPLACEMENTS_DEBOUNCE,
            Msg::Ui(UiMsg::ReplacementsDebounce(generation)),
        );
        self.ui.replacements.debounce = Some(timer);
    }

    /// Writes pending edits now: blank rows dropped, partial rows kept, and
    /// nothing written when the rules match the last load or write
    /// (REP-12).
    pub(super) fn flush_replacements(&mut self) {
        let editor = &mut self.ui.replacements;
        editor.debounce = None;
        if !std::mem::take(&mut editor.dirty) {
            return;
        }
        let rules = rules_to_persist(&editor.rules());
        if rules == editor.saved {
            return;
        }
        editor.saved = rules.clone();
        self.apply_setting(SettingChange::ReplacementRules(rules));
    }

    pub(super) fn replacements_view(&self) -> ReplacementsPage {
        let editor = &self.ui.replacements;
        replacements_page(
            editor
                .rows
                .iter()
                .map(|row| ReplacementRowView {
                    id: row.id.clone(),
                    triggers: row.triggers.clone(),
                    replacement: row.replacement.clone(),
                })
                .collect(),
            editor.revision,
        )
    }
}
