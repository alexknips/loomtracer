//! Resolve-by-description: the shared primitive behind every fluent follow-up verb.
//!
//! An agent that just ran a free-text search knows a decision's *content*, not an opaque id.
//! This module closes that gap deterministically — no LLM, no embeddings, no learned weights
//! (PRINCIPLES.md §1/§7) — by reusing `search.rs`'s existing rank-tier matcher
//! (`collect_resolver_candidates`) and adding `event_origin` (the ledger offset already on every
//! node) as a recency tiebreak. See docs/AGENT_FLUENT_QUERYING.md for the full design.
//!
//! Ambiguity is a value, not an error: `ResolveOutcome::Ambiguous` is the expected outcome when a
//! description matches more than one decision at the same confidence tier, matching this
//! project's stance that `contested` is a status, never a silently-collapsed error (AGENTS.md §6).
//! The gate for a description that matches in full is deliberately conservative. A verb that reads
//! resolves only when exactly one candidate occupies the best rank tier, and, among several that
//! hold every word, when it is about the question (see below). A verb that writes resolves only
//! when one decision alone holds every word, or the question names one outright (its id or title
//! is the question, or it records the question as asked): where the words sit orders its list but
//! never picks for it, since a rejection, a new title or a new premise cannot be taken back
//! (hivemind-293q).
//!
//! A description that names a word no decision contains ("why did we choose to move the demo
//! cell..." when the record never says "choose") is not a dead end: when no decision matches
//! every term, decisions matching most of them come back as an `Ambiguous` candidate list, each
//! carrying the terms it lacks. A verb that writes never auto-resolves a close candidate. A verb
//! that only reads (`why`, `verify`, ...) asks with `resolve_decision_for_reading`: it matches at
//! the bar `recall` uses, and answers with a close candidate that leads alone, saying what the
//! decision lacks, instead of making the asker repeat the question with `--pick`. Which candidate
//! leads is decided first by how much of the question its title, topic keys and recorded question
//! carry (`About`), then by how many terms it lacks (see `closeness`), so a decision about the
//! thing asked about beats one that only mentions the words somewhere in a long rationale, however
//! few words that one lacks. And a close candidate is answered with only when its own title and
//! topic keys hold at least `MIN_HEADLINE_WORDS_TO_ANSWER_CLOSE` of the question's words: one
//! shared word is what any decision on a broad subject has, and a decision that lacks the very
//! word the question is about is not the decision it asks about; it is listed.
//!
//! Holding every word is not being about the question, either: a decision whose long rationale
//! says all of a plain question's words matches in full, and a full match drops every close
//! candidate, so the decision whose title and topic keys carry the question but lacks one word
//! was never offered. A close candidate whose title and topic keys carry more of the question
//! (`About`) than every full match is promoted (`out_heading`): listed first, and answered with by
//! a verb that only reads, saying what it lacks. Among full matches, the one that is more about
//! the question comes first. It is answered with alone only when it is about the question in its
//! own right: with other decisions holding every word too, its own title and topic keys must hold
//! more than half of the question's words, or the full matches are listed (`full_match_leads`). A
//! leader that out-weighs the rest by one word in its title, with the others only in a long
//! rationale, is not the decision asked about because it leads by that word. A verb that writes
//! is answered with such a leader never: with other decisions holding every word, it gets the
//! list.
//!
//! A promoted close candidate is held to the bar of a full match that leads with rivals: it is
//! answered with alone only when its own title and topic keys hold three of the question's words
//! (all of them, in a shorter question), or the full matches it was promoted over are listed with
//! it; in a question of three words two are enough while no more than one other decision holds
//! every word and says one of them in its headline, as for a full match. No more than two
//! promoted candidates are listed before the first full match (hivemind-vecg4).
//!
//! More than half is not enough of a short question (hivemind-jis8). Of three words, two leave a
//! third to a long rationale that the other decisions on the same subject hold as well as the
//! leader does, so a leader with two or more such siblings among the full matches (they say a
//! word of the question in their own title or topic keys) also needs all three of the words in
//! its title and topic keys.
//!
//! The only decision that holds every word is not thereby the one asked about either: with one
//! word of a long question in its headline and the rest in its rationale ("if I rename a
//! decision, will old links to it break?" is not about the website rename that says "rename" in
//! its title), a verb that only reads lists it among the close candidates a full match used to
//! drop, unless two of the words are in its title and topic keys, in any form, or the question
//! names it outright. With no close candidate there is no one to list it among, and it is
//! answered with, as is the only decision a verb that writes finds (`thin_sole_full_match`).
//!
//! Common words are not told apart by the headline (hivemind-bnces). With
//! `FULL_MATCHES_THAT_MAKE_WORDS_COMMON` decisions or more holding every word, the leader is
//! answered with alone only if its own title and topic keys say a word of the question that none of
//! the other full matches say in theirs (`leader_names_a_word_alone`): a word that only one
//! headline names is the subject the question is about, and the words the headlines name in turns
//! are not. Otherwise the full matches are listed, the leader first. The decision the question
//! names outright keeps its answer; below that many full matches the bars above apply as before.
//!
//! A negation in the description ("don't adopt Kafka", "why didn't we ...", "do not ...") is not a
//! word to find: it never appears in `missing_terms`. It is polarity. A decision is the opposite of
//! a negated description when its own title says every word asked about outright, outside any
//! negation ("Adopt Kafka" for "why didn't we adopt Kafka"): it is a close candidate whose reason
//! is `POLARITY_REASON`, and is never resolved to, by a verb that writes or one that reads. A
//! title that leaves a word out, or denies it, is not the opposite of anything: it competes like
//! any other, on how much of the question it carries, so a negated question is not answered by
//! whatever decision happens to say "no" somewhere, nor sent past the decision asked about because
//! that decision states its answer positively.
//!
//! A record a later decision replaced is not a rival of the decision that replaced it
//! (hivemind-am6rp). When the outcome of a verb that only reads would be `Ambiguous`, every
//! candidate is folded into the decision at the end of its supersession chain
//! (`fold_into_chain_heads`) and the outcome is decided again over those: candidates that end in
//! one decision are one candidate, that decision, naming the records the words matched in
//! `resolved_via`; candidates that end in different decisions stay apart, each naming its own. A
//! chain that branches or loops has no single end and keeps its candidates as they are. A
//! description that already resolves keeps its answer, a replaced decision included, and a verb
//! that writes folds nothing: it never acts on a decision for the words of a record that decision
//! replaced.
//!
//! A decision is not a rival of the decisions that rest on it either (hivemind-zdsh.25). A
//! decision and its own consequences share its words: "Remove web/ from loomtracer now" follows
//! from "Split the frontend into its own repo" and says what that one says, so the decision the
//! question is about can never win by the margin of its words. When the outcome of a verb that
//! only reads is still `Ambiguous` after the replaced records are folded, every candidate that
//! rests on exactly one candidate that rests on none (`FOLLOWS_FROM`, directly or through other
//! decisions, a folded candidate standing for the records it absorbed) is folded into that one
//! (`fold_into_premises`), and the outcome is decided again over what is left. The root answers
//! with the words it matched itself, naming the candidates that rest on it in `consequences`: a
//! consequence's words are not the root's, so none are carried over. Candidates that rest on
//! nothing in common stay apart, as does one that rests on two roots; a loop of decisions that
//! rest on each other has no root and stays as it is. A description that already resolves keeps
//! its answer, and a verb that writes folds nothing.

use std::cmp::{Ordering, Reverse};
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::projector::{GraphParams, GraphValue, GraphView, NodeKind, RelationKind};
use crate::Result;

use super::follows_from::premises_reached;
use super::same_as::{fold_linked, RecordedCopy};
use super::search::{
    collect_resolver_candidates, out_heading, CloseMatch, ResolverCandidateRow, Standing,
    SAYS_AS_MUCH_SHARE,
};
use super::shared::{
    neighbor_pairs, normalized_topic_filter_values, optional_int, optional_string, query_error,
    query_timer_start, Direction, MAX_QUERY_RESULTS,
};
use super::status::{derive_decision_status, DecisionStatus};
use super::terms::resolver_terms;
use super::QueryResponse;

/// Fewest description words a close candidate must share before a read verb answers with it
/// instead of listing it. Listing needs only half the words (as `recall` does); answering names
/// the decision as the one asked about, and one shared word out of two is too little for that.
const MIN_WORDS_TO_ANSWER_CLOSE: usize = 2;

/// Fewest of the question's words a close candidate's own title and topic keys must hold, as
/// words, before a read verb answers with it. A decision that lacks some of the words and holds
/// fewer than this in its headline has the rest only in a long rationale, its evidence, the
/// question it records or its recorder's fields, or lacks the one word the question is about: it
/// is listed, never named as the decision asked about. The question a decision records orders it
/// (`About`) but does not count here: it can be as long as a paragraph, and a decision with a
/// long one holds nearly every common word as a word.
const MIN_HEADLINE_WORDS_TO_ANSWER_CLOSE: usize = 2;

/// Fewest of the question's words the title and topic keys of the only decision that holds every
/// word must contain, in any form, before a read verb answers with it while other decisions hold
/// most of them. The bar of a close candidate (`MIN_HEADLINE_WORDS_TO_ANSWER_CLOSE`), asked of a
/// full match too: a decision that holds every word of a long question only in its rationale, with
/// one word in its headline, is the one a long rationale holds a question's words in, not the one
/// the question is about. A question of fewer words needs all of them.
const MIN_HEADLINE_TERMS_TO_ANSWER_SOLE_FULL: usize = 2;

/// Fewest of the question's words the title and topic keys of the first of several decisions that
/// hold every word must hold, as words, before a read verb answers with it alone while
/// `RIVALS_THAT_RAISE_THE_BAR` or more of the others are on its subject. That is on top of more
/// than half of them: in a three-word question two is a majority and still leaves a third of the
/// question to a long rationale, which the decisions on the same subject hold as well as it does
/// (hivemind-jis8). A question of fewer words needs all of them.
const MIN_HEADLINE_WORDS_TO_LEAD_RIVALS: usize = 3;

/// How many other decisions that hold every word and say at least one of the question's words in
/// their own title or topic keys it takes to be on the subject the leader is: a cluster of
/// siblings, among which holding two of three words in the headline is a thin lead. Fewer than
/// that, two of three words stay a lead (a decision with one rival on its subject is told apart
/// by its title).
const RIVALS_THAT_RAISE_THE_BAR: usize = 2;

/// How many promoted close candidates (`out_heading`) a list shows before its first full match. A
/// weak full match is out-weighed by every close candidate whose title says a little more of the
/// question, and those can run to a dozen: the decision that holds every word then sits ninth,
/// where nobody looks (hivemind-vecg4). Two stay ahead of it, the rest follow it.
const PROMOTED_AHEAD_OF_FULL_MATCH: usize = 2;

/// How many decisions holding every word of a question make its words common ones. From this many
/// on, the decisions that hold every word are not told apart by who says most of the words in the
/// headline: a plain question finds that many decisions that each say one or two of its words in
/// their titles and the rest in their rationales, and the one that says most of them is as likely
/// to be about another subject as the question's own (hivemind-bnces).
const FULL_MATCHES_THAT_MAKE_WORDS_COMMON: usize = 4;

/// Why a decision that has every word asked is still only a close candidate: the description is
/// negated and the decision's title says all of it outright. Shown beside the candidate wherever
/// `missing_terms` would be, so a reader sees the opposite polarity instead of being told it lacks
/// "not".
pub const POLARITY_REASON: &str = "question is negated; this decision is not";

/// `serde` skip for a flag that is only worth saying when set.
fn is_false(value: &bool) -> bool {
    !*value
}

/// Who asks: a verb that acts on the decision it resolves, or one that only shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asker {
    /// `disagree`, `supersede`, `ground`, ...: a wrong pick writes to the wrong decision.
    Writer,
    /// `why`, `verify`, `chain`, `compact-view`: a wrong pick shows the wrong decision, and says
    /// what it lacks.
    Reader,
}

/// One candidate returned by the resolver, ordered by descending confidence
/// (ascending `rank`, then descending `event_origin` as the recency tiebreak).
///
/// `Deserialize` backs the CLI-layer `#N` continuation file (a local convenience cache, not
/// decision memory — see the `resolve_fluent_target` helper in `src/cli/run/mod.rs`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedCandidate {
    pub decision_id: String,
    pub title: String,
    /// Reused tier from `evaluate_search_match`; 0 = exact id, title or recorded-question match, lower is better.
    pub rank: u8,
    /// Ledger offset at creation — recency tiebreak, never a ranking input on its own.
    pub event_origin: i64,
    /// What the graph says became of the decision. Listed with every candidate so a replaced or
    /// contested decision is not offered as if it stood (hivemind-erlbl).
    pub status: DecisionStatus,
    /// The decisions that replaced it, by id; empty unless `status` is `superseded`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_by: Vec<String>,
    pub matched_fields: Vec<String>,
    /// Description terms this decision does not contain. Empty for a full match; non-empty only
    /// for the close candidates offered when no decision matches every term.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_terms: Vec<String>,
    /// The description is negated and this decision's title says every word of it outright
    /// (`POLARITY_REASON`). Such a decision is a close candidate even with `missing_terms` empty,
    /// and is never `Resolved`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub polarity_mismatch: bool,
    /// Other records of this same decision, linked `SAME_AS` (hivemind-83cj): the candidate is the
    /// earliest recorded of them that matches the description, and these are the rest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_recorded_as: Vec<RecordedCopy>,
    /// Records the description matched that this decision replaced (hivemind-am6rp): a verb that
    /// only reads folds a replaced record into the decision at the end of its supersession chain,
    /// so the candidate is that decision and these are the matched records that led to it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_via: Vec<RecordedCopy>,
    /// Candidates that rest on this decision (hivemind-zdsh.25): a verb that only reads folds a
    /// candidate that follows from another candidate into it, so the candidate is the decision
    /// the others follow from and these are the ones the description also matched, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consequences: Vec<RecordedCopy>,
}

impl ResolvedCandidate {
    /// Whether this is a close candidate rather than a full match: it lacks some of the words
    /// asked, or it is the opposite of what a negated description asked for.
    pub fn is_close(&self) -> bool {
        !self.missing_terms.is_empty() || self.polarity_mismatch
    }

    /// Whether an answer resolved to this candidate has something to say about how it was
    /// resolved: words it lacks, other records of the same decision, the replaced records it
    /// was reached through, or the decisions that rest on it.
    pub fn needs_annotation(&self) -> bool {
        !self.missing_terms.is_empty()
            || !self.also_recorded_as.is_empty()
            || !self.resolved_via.is_empty()
            || !self.consequences.is_empty()
    }
}

/// Outcome of a resolve-by-description call: either a confident single match, or a candidate
/// list the caller must disambiguate via `--pick N` / `#N` or the `--id` escape hatch.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ResolveOutcome {
    Resolved { candidate: ResolvedCandidate },
    Ambiguous { candidates: Vec<ResolvedCandidate> },
    NotFound,
}

/// Resolve a free-text description to a decision, for a verb that writes to it. `topic_hint`
/// narrows the candidate set the same way `search --topic` / `get_relevant_decisions --topic`
/// already do.
///
/// Resolved iff exactly one decision matches every term, or the description names one outright
/// (its id or title is the description, or it records the description as asked): with several
/// that hold every word, where the words sit orders the list but never picks, so the call writes
/// nothing and lists them (hivemind-293q). There is no numeric recency margin (mayor decision,
/// hivemind-tenv.1, 2026-09-07). Close candidates (missing some terms) are offered, as
/// `Ambiguous`, only when there is no full match, and never resolved: more than half of the
/// terms, and at least two.
pub fn resolve_decision_by_description(
    graph: &impl GraphView,
    description: &str,
    topic_hint: Option<&str>,
) -> Result<QueryResponse<ResolveOutcome>> {
    resolve_for(graph, description, topic_hint, Asker::Writer)
}

/// Resolve a free-text description to a decision, for a verb that only shows it (`why`,
/// `verify`, `chain`, `compact-view`). Full matches resolve exactly as for a writer. The two
/// differences are for a description no decision matches in full (hivemind-3lko):
///
/// - a close candidate needs at least half of the terms, the bar `recall` uses, so a question
///   `recall` answers is not answered with "no decision matches" here;
/// - when close candidates are all there is, and the one that is most about the question is also
///   closer than the rest (lacks fewer terms, or lacks as many but has more of the terms it
///   matched in its title or topic keys), shares at least two terms and holds at least two of them
///   in its own title or topic keys, it is `Resolved` with its `missing_terms` set, so the caller
///   shows the decision and names what it lacks. Equally close candidates, and a candidate whose
///   headline holds fewer than two of the words, stay an `Ambiguous` list (hivemind-tfde).
///
/// When that would be `Ambiguous`, the records the words matched that a later decision replaced
/// are folded into the decision at the end of their supersession chain and the outcome is decided
/// again over the decisions that stand, each naming the replaced records it absorbed in
/// `resolved_via` (hivemind-am6rp). If it is still `Ambiguous`, the candidates that rest on exactly
/// one other candidate that rests on none are folded into it and the outcome is decided once more,
/// the root naming them in `consequences` (hivemind-zdsh.25).
///
/// And for a description one decision alone matches in full, when that decision says fewer than two
/// of the words in its title and topic keys and close candidates exist: it is `Ambiguous`, listed
/// first, with the close candidates after it, so the decision that is about the question is not
/// hidden behind a confident answer about another (hivemind-jis8).
pub fn resolve_decision_for_reading(
    graph: &impl GraphView,
    description: &str,
    topic_hint: Option<&str>,
) -> Result<QueryResponse<ResolveOutcome>> {
    resolve_for(graph, description, topic_hint, Asker::Reader)
}

fn resolve_for(
    graph: &impl GraphView,
    description: &str,
    topic_hint: Option<&str>,
    asker: Asker,
) -> Result<QueryResponse<ResolveOutcome>> {
    let started = query_timer_start();
    let description = description.trim();
    if description.is_empty() {
        return Err(query_error("description must not be empty").into());
    }

    let topic_keys: Vec<String> = topic_hint
        .map(|topic| normalized_topic_filter_values(&[topic.to_owned()]))
        .unwrap_or_default();

    let close = match asker {
        Asker::Writer => CloseMatch::Majority,
        Asker::Reader => CloseMatch::Half,
    };
    // A verb that only shows a decision reads the description as `recall` does, so it names what
    // `recall` just named: a word's other forms and the words that stand in for it. A verb that
    // writes matches a word and its inflections only; a synonym never picks what it writes to.
    let related = asker == Asker::Reader;
    let found = collect_resolver_candidates(graph, description, &topic_keys, close, related)?;
    // A verb that only shows a decision answers with the one that stands when the records the
    // words matched would otherwise be listed against it: if the outcome is `Ambiguous`, the
    // candidates are folded into the heads of their supersession chains and the outcome is
    // decided again over those. A description that already resolves keeps its answer, so asking
    // about a decision that was replaced still answers with it, saying it was replaced. A verb
    // that writes keeps every record apart; it never acts on a decision whose own words were not
    // the ones asked (hivemind-am6rp).
    //
    // What is still `Ambiguous` then may be a decision and the decisions that rest on it, which
    // say what it says. Those are folded into it as its consequences, and the outcome is decided
    // once more over the decisions that rest on nothing in the list (hivemind-zdsh.25).
    let unfolded = (asker == Asker::Reader).then(|| found.clone());
    let mut settled = settle(graph, description, asker, found)?;
    if let Some(mut rows) = unfolded {
        if matches!(settled.data, ResolveOutcome::Ambiguous { .. }) {
            rows = fold_into_chain_heads(graph, rows)?;
            if rows.iter().any(|row| !row.resolved_via.is_empty()) {
                settled = settle(graph, description, asker, rows.clone())?;
            }
        }
        if matches!(settled.data, ResolveOutcome::Ambiguous { .. }) {
            let roots = fold_into_premises(graph, rows)?;
            if roots.iter().any(|row| !row.consequences.is_empty()) {
                settled = settle(graph, description, asker, roots)?;
            }
        }
    }
    settled.latency_ms = started.elapsed().as_millis();
    Ok(settled)
}

/// What `description` resolves to over the candidates `found`, for `asker`. The latency of the
/// response it returns is not set: the caller measures the whole call.
fn settle(
    graph: &impl GraphView,
    description: &str,
    asker: Asker,
    found: Vec<ResolverCandidateRow>,
) -> Result<QueryResponse<ResolveOutcome>> {
    // Records of one decision, linked `SAME_AS`, are one candidate; records with no link between
    // them stay separate, so a tie among them stays `Ambiguous`.
    let mut also_recorded_as: HashMap<String, Vec<RecordedCopy>> = HashMap::new();
    let mut rows: Vec<ResolverCandidateRow> = Vec::new();
    for folded in fold_linked(graph, found, |row| row.decision_id.as_str(), absorb_record)? {
        if !folded.also_recorded_as.is_empty() {
            also_recorded_as.insert(folded.item.decision_id.clone(), folded.also_recorded_as);
        }
        rows.push(folded.item);
    }
    let only_close_candidates = !rows.is_empty() && rows.iter().all(ResolverCandidateRow::is_close);
    let standings: Vec<Standing> = rows
        .iter()
        .map(|row| Standing {
            missing: row.missing_terms.len(),
            opposite: row.polarity_mismatch,
            about: row.about,
        })
        .collect();
    for (row, promoted) in rows.iter_mut().zip(out_heading(&standings)) {
        row.promoted = promoted;
    }
    // The close candidates a full match drops are kept aside: when the only full match turns out
    // not to be the one asked about, they are what it is listed among.
    let mut held_back: Vec<ResolverCandidateRow> = Vec::new();
    if !only_close_candidates {
        let (kept, dropped): (Vec<_>, Vec<_>) = rows
            .into_iter()
            .partition(|row| row.promoted || !row.is_close());
        rows = kept;
        held_back = dropped;
    }
    rows.sort_by(by_standing);
    let term_count = resolver_terms(description).len();
    let sole_match_is_thin =
        asker == Asker::Reader && !held_back.is_empty() && thin_sole_full_match(term_count, &rows);
    if sole_match_is_thin {
        rows.extend(held_back);
        rows.sort_by(by_standing_filed);
    }

    let truncated = rows.len() > MAX_QUERY_RESULTS;
    rows.truncate(MAX_QUERY_RESULTS);
    lift_first_full_match(&mut rows);

    // The leader's ties among the full matches: the rows that hold as much of what the question
    // is about, lean on no more stand-in words and sit in the same rank tier.
    let leader_ties = rows.first().map_or(0, |leader| {
        rows.iter()
            .filter(|row| {
                row.about.cmp_about(&leader.about).is_eq()
                    && row.stand_in_terms == leader.stand_in_terms
                    && row.rank == leader.rank
            })
            .count()
    });
    let leader_is_clear = asker == Asker::Reader && close_candidate_leads(term_count, &rows);
    let full_leader_is_about_it = !sole_match_is_thin && full_match_leads(asker, term_count, &rows);

    let leader_promoted = rows.first().is_some_and(|leader| leader.promoted);
    let mut candidates: Vec<ResolvedCandidate> = rows
        .into_iter()
        .map(|row| ResolvedCandidate {
            also_recorded_as: also_recorded_as
                .remove(&row.decision_id)
                .unwrap_or_default(),
            decision_id: row.decision_id,
            title: row.title,
            rank: row.rank,
            event_origin: row.event_origin,
            status: row.status,
            superseded_by: row.superseded_by,
            matched_fields: row.matched_fields,
            missing_terms: row.missing_terms,
            polarity_mismatch: row.polarity_mismatch,
            resolved_via: row.resolved_via,
            consequences: row.consequences,
        })
        .collect();

    let result_count = candidates.len();
    let outcome = if only_close_candidates || leader_promoted {
        if leader_is_clear {
            ResolveOutcome::Resolved {
                candidate: candidates.remove(0),
            }
        } else {
            ResolveOutcome::Ambiguous { candidates }
        }
    } else if !candidates.is_empty() {
        if leader_ties == 1 && full_leader_is_about_it {
            ResolveOutcome::Resolved {
                candidate: candidates.remove(0),
            }
        } else {
            ResolveOutcome::Ambiguous { candidates }
        }
    } else {
        ResolveOutcome::NotFound
    };

    Ok(QueryResponse {
        result_count,
        truncated,
        latency_ms: 0,
        data: outcome,
    })
}

/// The order of a list and the pick of a verb that only reads: a close candidate that
/// `out_heading` promoted, then the decisions that hold every word, then the one more about the
/// question (`About`), the one that lacks fewer words, the one whose headline carries more of the
/// words it matched, the one that leans on fewer stand-ins, then the rank tier, then an accepted
/// decision before a proposed draft, recency last.
fn by_standing(left: &ResolverCandidateRow, right: &ResolverCandidateRow) -> Ordering {
    standing_before_recency(left, right)
        .then_with(|| right.accepted.cmp(&left.accepted))
        .then_with(|| right.event_origin.cmp(&left.event_origin))
        .then_with(|| left.decision_id.cmp(&right.decision_id))
}

/// `by_standing`, except that between candidates that tie on everything but recency, the one that
/// names a word in its title and its topic keys both comes first: it is filed under what it says
/// it is about. The order of the list a thin sole full match heads, whose close candidates are
/// mostly the decisions that say one word of the question and hold a few more somewhere in their
/// rationale, and that recency alone would put in any order.
fn by_standing_filed(left: &ResolverCandidateRow, right: &ResolverCandidateRow) -> Ordering {
    standing_before_recency(left, right)
        .then_with(|| right.headline_hits.cmp(&left.headline_hits))
        .then_with(|| right.accepted.cmp(&left.accepted))
        .then_with(|| right.event_origin.cmp(&left.event_origin))
        .then_with(|| left.decision_id.cmp(&right.decision_id))
}

fn standing_before_recency(left: &ResolverCandidateRow, right: &ResolverCandidateRow) -> Ordering {
    right
        .promoted
        .cmp(&left.promoted)
        .then_with(|| {
            right
                .missing_terms
                .is_empty()
                .cmp(&left.missing_terms.is_empty())
        })
        .then_with(|| right.about.cmp_about(&left.about))
        .then_with(|| left.missing_terms.len().cmp(&right.missing_terms.len()))
        .then_with(|| right.headline_terms.cmp(&left.headline_terms))
        .then_with(|| left.stand_in_terms.cmp(&right.stand_in_terms))
        .then_with(|| left.rank.cmp(&right.rank))
}

/// How many of the decisions after the first of `rows` hold every word and say at least one of
/// the question's words in their own title or topic keys: the siblings the leader has to be told
/// apart from.
fn rivals_on_the_subject(rows: &[ResolverCandidateRow]) -> usize {
    rows.iter()
        .skip(1)
        .filter(|row| !row.is_close() && row.headline_words >= 1)
        .count()
}

/// Whether the one decision that holds every word says too little of the question in its own title
/// and topic keys to be named as the decision asked about, for a verb that only reads
/// (`MIN_HEADLINE_TERMS_TO_ANSWER_SOLE_FULL`). Such a decision holds the words the way a long
/// rationale does, and "if I rename a decision, will old links to it break?" is not about the
/// website rename that says "rename" in its title and "old links break" in its rationale. The
/// decision the question names outright is the one asked about whatever its words.
fn thin_sole_full_match(term_count: usize, rows: &[ResolverCandidateRow]) -> bool {
    let [only] = rows else {
        return false;
    };
    !only.is_close()
        && !only.promoted
        && !only.about.is_named()
        && only.headline_matched < term_count.min(MIN_HEADLINE_TERMS_TO_ANSWER_SOLE_FULL)
}

/// How far a candidate is from the description, smaller being closer: it lacks fewer of the
/// terms, and between two that lack the same number, the one whose title or topic keys carry more
/// of the terms it matched, then the one that matched fewer of them only through a stand-in word
/// (a decision that says "agent" is closer to a question about an agent than one that says "bot").
/// A decision about the product's name says "product" and "Upheld" in its title; one that happens
/// to say "product" and "called" somewhere in a long rationale is not about that, though it lacks
/// just as many words. Where the words matched is already in `matched_fields`; nothing is counted
/// across the ledger and nothing is learned. A full match is 0 on the headline count (its order
/// is `About`, then the stand-in count and the rank tier), and a verb that writes never reads
/// stand-ins, so it is 0 on the last one too.
///
/// This is how close the candidate is, which says whether the first of a list leads it clearly.
/// The order of the list is decided before it: by `About`, so a decision whose title says the
/// question comes ahead of one that lacks a word fewer and holds the others only in its rationale.
fn closeness(row: &ResolverCandidateRow) -> (usize, Reverse<usize>, usize) {
    (
        row.missing_terms.len(),
        Reverse(row.headline_terms),
        row.stand_in_terms,
    )
}

/// Moves the first full match up to just behind `PROMOTED_AHEAD_OF_FULL_MATCH` promoted close
/// candidates, so a list never buries the decisions that hold every word the asker used behind a
/// long run of decisions that lack one of them (hivemind-vecg4). Only the order of a list changes:
/// what a call resolves to is decided on the first two rows, which stay where they are.
fn lift_first_full_match(rows: &mut Vec<ResolverCandidateRow>) {
    if let Some(at) = rows.iter().position(|row| !row.is_close()) {
        if at > PROMOTED_AHEAD_OF_FULL_MATCH {
            let full = rows.remove(at);
            rows.insert(PROMOTED_AHEAD_OF_FULL_MATCH, full);
        }
    }
}

/// Whether `first`, a close candidate that out-weighs the decisions holding every word (it is
/// promoted), says enough of the question in its own title and topic keys to be named as the
/// decision asked about: `MIN_HEADLINE_WORDS_TO_LEAD_RIVALS` of the words (all of them, in a shorter
/// question), the bar of a full match that leads with rivals. In a question of three words, two
/// stay enough while fewer than `RIVALS_THAT_RAISE_THE_BAR` of the decisions that hold every word
/// are on the subject, as jis8 holds a full match there (hivemind-vecg4).
fn promoted_leader_is_about_it(
    term_count: usize,
    rows: &[ResolverCandidateRow],
    first: &ResolverCandidateRow,
) -> bool {
    first.headline_words >= term_count.min(MIN_HEADLINE_WORDS_TO_LEAD_RIVALS)
        || (first.headline_words * 2 > term_count
            && rivals_on_the_subject(rows) < RIVALS_THAT_RAISE_THE_BAR)
}

/// Whether the first of `rows` (a close candidate, the most about the question first) is the one
/// asked about: its title and topic keys hold enough of the question's words
/// (`MIN_HEADLINE_WORDS_TO_ANSWER_CLOSE`), it shares enough terms to be named as the answer, and it
/// is closer than the next, by `closeness`, or the next is a decision it was promoted over
/// (`out_heading`). A first that is the most about the question but lacks more words than the next
/// is not resolved, and neither is an equally close one: after it the lists are ordered by rank
/// and recency, which say nothing about which of two decisions was meant. A decision of the
/// opposite polarity is never the one asked about, however many words it shares.
///
/// A promoted first has out-weighed decisions that hold every word, so it is held to the bar of a
/// full match that leads with rivals (`promoted_leader_is_about_it`): two generic words of its
/// title and topic keys, with the word the question is about only in the rationale of the others,
/// is not the decision asked about because it out-weighs them (hivemind-vecg4).
fn close_candidate_leads(term_count: usize, rows: &[ResolverCandidateRow]) -> bool {
    let Some(first) = rows.first() else {
        return false;
    };
    let shared = term_count.saturating_sub(first.missing_terms.len());
    !first.polarity_mismatch
        && !first.peer
        && !first.outweighed
        && shared >= MIN_WORDS_TO_ANSWER_CLOSE
        && first.headline_words >= MIN_HEADLINE_WORDS_TO_ANSWER_CLOSE
        && (!first.promoted || promoted_leader_is_about_it(term_count, rows, first))
        && rows.get(1).is_none_or(|next| {
            (first.promoted && !next.promoted) || closeness(first) < closeness(next)
        })
        && rows
            .get(1)
            .is_none_or(|next| !says_as_much_in_its_title(first, next))
}

/// Whether `next`, a close candidate, says as much of the question in its own title and topic keys
/// as `first` does: no fewer of its words, and as much of the question (`SAYS_AS_MUCH_SHARE` of
/// what `first` weighs, `About`). Lacking fewer words is then all `first` has over it, and a decision
/// that holds more of the words in a rationale is not the one asked about when a title says as
/// much (the graph that "draws" and the one that "makes" the call, hivemind-wdxx0).
fn says_as_much_in_its_title(first: &ResolverCandidateRow, next: &ResolverCandidateRow) -> bool {
    next.is_close()
        && next.headline_words >= first.headline_words
        && next.about.weighs_at_least(SAYS_AS_MUCH_SHARE, &first.about)
}

fn full_match_count(rows: &[ResolverCandidateRow]) -> usize {
    rows.iter().filter(|row| !row.is_close()).count()
}

/// Whether the first of `rows` says, in its own title or topic keys, a word of the question that
/// none of the other decisions holding every word says there: the one thing that sets it apart
/// from them when every word is common. A word only one of them names in its headline is the
/// subject the question is about; the words they each name in turns are not. The others count
/// for any way of saying it (another form of the word, a stand-in, a part of a longer word): a
/// title that says "shown" does not leave "show" to the title that says "showing".
fn leader_names_a_word_alone(rows: &[ResolverCandidateRow]) -> bool {
    let Some((first, rest)) = rows.split_first() else {
        return false;
    };
    let named_by_others = rest
        .iter()
        .filter(|row| !row.is_close())
        .fold(0, |mask, row| mask | row.headline_any_mask);
    first.headline_mask & !named_by_others != 0
}

/// Whether the first of `rows` (full matches, the most about the question first) is the one asked
/// about, so that a call answers with it alone. When other decisions hold every word too, a leader
/// that out-weighs them (`About`) can do it by one word in its title or topic keys, with no more
/// than half of the question there and the rest only in a long rationale: it is not thereby the
/// decision asked about, and naming it hides the others behind a confident answer (hivemind-5ctc).
/// So it is answered with only when its own title and topic keys hold more than half of the
/// question's words; and, when at least `RIVALS_THAT_RAISE_THE_BAR` other full matches are on its
/// subject too, `MIN_HEADLINE_WORDS_TO_LEAD_RIVALS` of them (all of them, in a shorter question:
/// two of three words is a majority and still a third of the question left to a rationale that
/// its siblings hold as well, hivemind-jis8); and, with `FULL_MATCHES_THAT_MAKE_WORDS_COMMON` full
/// matches or more, a word of the question that none of the others says in its own headline
/// (`leader_names_a_word_alone`, hivemind-bnces); otherwise the full matches are listed, most
/// about the question first. The only decision that holds every word has none to be listed among, and the decision
/// the question names outright (its id or title is the question, or it records the question as
/// asked) is the one asked about whatever its words: both are answered with as before, the only
/// one unless it is a thin match with close candidates to be listed among
/// (`thin_sole_full_match`). A close candidate has its own gate (`close_candidate_leads`).
///
/// A verb that writes is held to a stricter gate: where the words sit orders its list but never
/// picks for it (hivemind-293q). A wrong answer for a verb that shows is a wrong decision shown;
/// for one that writes it is a rejection, a new title or a new premise on the record of a decision
/// nobody meant, which append-only storage cannot take back, and the help of every such verb says
/// a description that matches more than one decision lists the candidates and writes nothing. So
/// with other decisions holding every word too, a writer is answered with the leader only when
/// the question names it outright; the only decision that holds every word is picked as before.
fn full_match_leads(asker: Asker, term_count: usize, rows: &[ResolverCandidateRow]) -> bool {
    let Some(first) = rows.first() else {
        return false;
    };
    match asker {
        Asker::Writer => rows.len() == 1 || first.about.is_named(),
        Asker::Reader => {
            rows.len() == 1
                || first.about.is_named()
                || (first.headline_words * 2 > term_count
                    && (first.headline_words >= term_count.min(MIN_HEADLINE_WORDS_TO_LEAD_RIVALS)
                        || rivals_on_the_subject(rows) < RIVALS_THAT_RAISE_THE_BAR)
                    && (full_match_count(rows) < FULL_MATCHES_THAT_MAKE_WORDS_COMMON
                        || leader_names_a_word_alone(rows)))
        }
    }
}

/// Takes the match of another record of the same decision onto the one shown: the best rank,
/// accepted if either record is, the fields either matched, only the words neither record lacks,
/// and the opposite polarity only when neither record says what was asked.
fn absorb_record(shown: &mut ResolverCandidateRow, other: ResolverCandidateRow) {
    shown.rank = shown.rank.min(other.rank);
    shown.accepted |= other.accepted;
    shown.stand_in_terms = shown.stand_in_terms.min(other.stand_in_terms);
    if other.headline_words > shown.headline_words {
        shown.headline_mask = other.headline_mask;
    }
    shown.headline_words = shown.headline_words.max(other.headline_words);
    shown.headline_matched = shown.headline_matched.max(other.headline_matched);
    shown.headline_any_mask |= other.headline_any_mask;
    shown.headline_hits = shown.headline_hits.max(other.headline_hits);
    if other.about.cmp_about(&shown.about).is_gt() {
        shown.about = other.about;
    }
    shown.polarity_mismatch &= other.polarity_mismatch;
    for field in other.matched_fields {
        if !shown.matched_fields.contains(&field) {
            shown.matched_fields.push(field);
        }
    }
    shown
        .missing_terms
        .retain(|term| other.missing_terms.contains(term));
    shown.resolved_via.extend(other.resolved_via);
    shown.consequences.extend(other.consequences);
}

/// Folds every candidate that a later decision replaced into the decision at the end of its
/// supersession chain (hivemind-am6rp). A question that matches an early draft, or a decision
/// that was reworded and then replaced, finds records that all lead to the one decision that
/// stands, and listing them as rivals answers "ambiguous" about a ledger that holds one answer.
/// Called only for a verb that only reads, once its outcome is `Ambiguous`.
///
/// The records that fold to one head become one candidate: the head itself when it matched (the
/// replaced records are absorbed like the records of one decision, `absorb_record`), otherwise the
/// best of the records, standing in for the head under the head's id, title, status and recorded
/// order, with the words it matched. Either way `resolved_via` names the replaced records that
/// led there. Records that fold to different heads stay different candidates, each with its own
/// `resolved_via`, so the outcome stays `Ambiguous` and lists the heads. A chain that branches (a
/// decision replaced by two) or loops has no head to fold into: its candidates stay as they are,
/// listed as replaced, as before.
fn fold_into_chain_heads(
    graph: &impl GraphView,
    rows: Vec<ResolverCandidateRow>,
) -> Result<Vec<ResolverCandidateRow>> {
    let mut groups: Vec<(Option<String>, Vec<ResolverCandidateRow>)> = Vec::new();
    for row in rows {
        let head = chain_head(graph, &row)?;
        let group = head
            .as_ref()
            .and_then(|head| groups.iter_mut().find(|(at, _)| at.as_ref() == Some(head)));
        match group {
            Some((_, members)) => members.push(row),
            None => groups.push((head, vec![row])),
        }
    }

    let mut folded = Vec::with_capacity(groups.len());
    for (head, members) in groups {
        match head {
            Some(head)
                if members.len() > 1 || members.iter().all(|row| row.decision_id != head) =>
            {
                folded.extend(fold_into_head(graph, &head, members)?);
            }
            _ => folded.extend(members),
        }
    }
    Ok(folded)
}

/// The decision at the end of the supersession chain `row` sits in: its own id when nothing
/// replaced it. `None` when the chain has no single end to fold into: a decision on the way was
/// replaced by two, or the chain loops back on itself.
fn chain_head(graph: &impl GraphView, row: &ResolverCandidateRow) -> Result<Option<String>> {
    if row.superseded_by.is_empty() {
        return Ok(Some(row.decision_id.clone()));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut at = row.decision_id.clone();
    loop {
        let mut replaced_by: Vec<String> = neighbor_pairs(
            graph,
            NodeKind::Decision,
            &at,
            RelationKind::Supersedes,
            NodeKind::Decision,
            Direction::Incoming,
        )?
        .into_iter()
        .map(|(by_id, _)| by_id)
        .collect();
        replaced_by.sort();
        replaced_by.dedup();
        let next = match replaced_by.len() {
            0 => return Ok(Some(at)),
            1 => replaced_by.swap_remove(0),
            _ => return Ok(None),
        };
        if next == at || seen.contains(&next) {
            return Ok(None);
        }
        seen.insert(std::mem::replace(&mut at, next));
    }
}

/// One candidate for the records `members` that end in `head`: see `fold_into_chain_heads`.
fn fold_into_head(
    graph: &impl GraphView,
    head: &str,
    members: Vec<ResolverCandidateRow>,
) -> Result<Vec<ResolverCandidateRow>> {
    let head_matched = members.iter().any(|row| row.decision_id == head);
    let standing = if head_matched {
        None
    } else {
        match resolve_decision_by_id(graph, head)?.data {
            ResolveOutcome::Resolved { candidate } => Some(candidate),
            _ => return Ok(members),
        }
    };

    let (matched_head, mut replaced): (Vec<_>, Vec<_>) =
        members.into_iter().partition(|row| row.decision_id == head);
    replaced.sort_by(by_standing);
    let mut replaced = replaced.into_iter();
    let Some(mut carrier) = matched_head.into_iter().next().or_else(|| replaced.next()) else {
        return Ok(Vec::new());
    };

    let mut led_here: Vec<(i64, RecordedCopy)> = Vec::new();
    let mut note = |row: &ResolverCandidateRow| {
        led_here.push((
            row.event_origin,
            RecordedCopy {
                decision_id: row.decision_id.clone(),
                title: row.title.clone(),
            },
        ));
    };
    if standing.is_some() {
        note(&carrier);
    }
    for other in replaced {
        note(&other);
        carrier.peer &= other.peer;
        carrier.outweighed &= other.outweighed;
        absorb_record(&mut carrier, other);
    }
    if let Some(standing) = standing {
        carrier.decision_id = standing.decision_id;
        carrier.title = standing.title;
        carrier.event_origin = standing.event_origin;
        carrier.accepted = standing.status == DecisionStatus::Accepted;
        carrier.status = standing.status;
        carrier.superseded_by = standing.superseded_by;
    }
    led_here.sort_by(|(left_at, left), (right_at, right)| {
        left_at
            .cmp(right_at)
            .then_with(|| left.decision_id.cmp(&right.decision_id))
    });
    carrier
        .resolved_via
        .extend(led_here.into_iter().map(|(_, record)| record));
    Ok(vec![carrier])
}

/// The ids a candidate answers for: its own, and those of the replaced records folded into it.
fn stands_for(row: &ResolverCandidateRow) -> impl Iterator<Item = &str> {
    std::iter::once(row.decision_id.as_str()).chain(
        row.resolved_via
            .iter()
            .map(|record| record.decision_id.as_str()),
    )
}

/// Folds each candidate that rests on another candidate into it (hivemind-zdsh.25). A decision and
/// the decisions that follow from it say the same words, so listing them as rivals answers
/// "ambiguous" about a ledger whose answer is the first of them. Called only for a verb that only
/// reads, once its outcome is still `Ambiguous`.
///
/// A candidate rests on another when that one is among the decisions it follows from, directly or
/// through other decisions (`FOLLOWS_FROM`); a candidate that absorbed replaced records stands for
/// them too, so a decision that rests on a replaced record rests on the one that replaced it. A
/// root is a candidate that rests on no other candidate. A candidate that rests on exactly one
/// root becomes one of its `consequences`, oldest first, and its own words are not carried over:
/// they are not the words the root matched. A candidate that rests on two roots, or only on
/// candidates that rest on each other, has no one decision to fold into and stays as it is, and so
/// do the roots.
fn fold_into_premises(
    graph: &impl GraphView,
    rows: Vec<ResolverCandidateRow>,
) -> Result<Vec<ResolverCandidateRow>> {
    // The candidates, by position, that each candidate rests on. A decision that names no
    // premise rests on nothing, which costs no walk.
    let mut rests_on: Vec<Vec<usize>> = Vec::with_capacity(rows.len());
    for (at, row) in rows.iter().enumerate() {
        if row.premises.is_empty() {
            rests_on.push(Vec::new());
            continue;
        }
        let premises = premises_reached(graph, &row.premises)?;
        rests_on.push(
            rows.iter()
                .enumerate()
                .filter(|(other_at, other)| {
                    *other_at != at && stands_for(other).any(|id| premises.contains(id))
                })
                .map(|(other_at, _)| other_at)
                .collect(),
        );
    }

    let root_of: Vec<Option<usize>> = rests_on
        .iter()
        .map(|premises| {
            let mut roots = premises
                .iter()
                .copied()
                .filter(|at| rests_on.get(*at).is_some_and(Vec::is_empty));
            match (roots.next(), roots.next()) {
                (Some(root), None) => Some(root),
                _ => None,
            }
        })
        .collect();

    let mut led_here: HashMap<usize, Vec<(i64, RecordedCopy)>> = HashMap::new();
    let mut kept: Vec<(usize, ResolverCandidateRow)> = Vec::new();
    for (at, (row, root)) in rows.into_iter().zip(root_of).enumerate() {
        match root {
            Some(root) => led_here.entry(root).or_default().push((
                row.event_origin,
                RecordedCopy {
                    decision_id: row.decision_id,
                    title: row.title,
                },
            )),
            None => kept.push((at, row)),
        }
    }
    Ok(kept
        .into_iter()
        .map(|(at, mut row)| {
            if let Some(mut rest_on_it) = led_here.remove(&at) {
                rest_on_it.sort_by(|(left_at, left), (right_at, right)| {
                    left_at
                        .cmp(right_at)
                        .then_with(|| left.decision_id.cmp(&right.decision_id))
                });
                row.consequences
                    .extend(rest_on_it.into_iter().map(|(_, record)| record));
            }
            row
        })
        .collect())
}

/// Look up a decision named by its literal id, as a grounding premise: `Resolved` when the
/// decision exists, `NotFound` when it does not. Same outcome shape as a description lookup so a
/// caller resolving a mixed list of ids and descriptions handles one type; the candidate carries
/// the decision's title so the caller can echo what it linked to.
pub fn resolve_decision_by_id(
    graph: &impl GraphView,
    decision_id: &str,
) -> Result<QueryResponse<ResolveOutcome>> {
    let started = query_timer_start();
    let decision_id = decision_id.trim();
    if decision_id.is_empty() {
        return Err(query_error("decision id must not be empty").into());
    }

    let rows = graph.query(
        "MATCH (d:`Decision` {id: $id}) RETURN d.id AS id, d.title AS title, d.event_origin AS event_origin LIMIT 1;",
        &GraphParams::from([("id".to_owned(), GraphValue::String(decision_id.to_owned()))]),
    )?;
    let data = match rows.first() {
        Some(row) => ResolveOutcome::Resolved {
            candidate: ResolvedCandidate {
                decision_id: decision_id.to_owned(),
                title: optional_string(row, "title").unwrap_or_default(),
                rank: 0,
                event_origin: optional_int(row, "event_origin").unwrap_or(0),
                status: derive_decision_status(graph, decision_id)?,
                superseded_by: neighbor_pairs(
                    graph,
                    NodeKind::Decision,
                    decision_id,
                    RelationKind::Supersedes,
                    NodeKind::Decision,
                    Direction::Incoming,
                )?
                .into_iter()
                .map(|(by_id, _)| by_id)
                .collect(),
                matched_fields: vec!["id".to_owned()],
                missing_terms: Vec::new(),
                polarity_mismatch: false,
                also_recorded_as: Vec::new(),
                resolved_via: Vec::new(),
                consequences: Vec::new(),
            },
        },
        None => ResolveOutcome::NotFound,
    };

    Ok(QueryResponse {
        result_count: usize::from(matches!(data, ResolveOutcome::Resolved { .. })),
        truncated: false,
        latency_ms: started.elapsed().as_millis(),
        data,
    })
}

/// Says, beside `data` in a read verb's JSON envelope, how a description was resolved when that
/// is worth knowing: `close_match: {decision_id, title, missing_terms}` for a decision that
/// matched all but some of the words asked, `also_recorded_as: [{decision_id, title}]` for the
/// other records of a decision recorded more than once, `resolved_via: [{decision_id, title}]`
/// for the replaced records the description matched that led to the decision that stands, and
/// `consequences: [{decision_id, title}]` for the matched decisions that rest on it. A
/// description that matched in full a decision recorded once adds nothing, so a key's presence is
/// the whole signal. Shared by the CLI's `--json`, the HTTP API and MCP so the three cannot name
/// it differently.
pub fn annotate_resolution(envelope: &mut serde_json::Value, candidate: &ResolvedCandidate) {
    let Some(envelope) = envelope.as_object_mut() else {
        return;
    };
    if !candidate.missing_terms.is_empty() {
        envelope.insert(
            "close_match".to_owned(),
            serde_json::json!({
                "decision_id": candidate.decision_id,
                "title": candidate.title,
                "missing_terms": candidate.missing_terms,
            }),
        );
    }
    if !candidate.also_recorded_as.is_empty() {
        envelope.insert(
            "also_recorded_as".to_owned(),
            serde_json::json!(candidate.also_recorded_as),
        );
    }
    if !candidate.resolved_via.is_empty() {
        envelope.insert(
            "resolved_via".to_owned(),
            serde_json::json!(candidate.resolved_via),
        );
    }
    if !candidate.consequences.is_empty() {
        envelope.insert(
            "consequences".to_owned(),
            serde_json::json!(candidate.consequences),
        );
    }
}

#[cfg(test)]
mod tests;
