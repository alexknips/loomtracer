/** Independently authored v5 synthetic workflows. No provider is called here. */
export const CORPUS_VERSION = 'functionality-v5-corpus-1.0.3';
export const selectedScreenIds = Object.freeze([
  'v5-incident-demanding-same',
  'v5-experiment-demanding-distinct',
  'v5-release-demanding-unresolved',
]);
export const H_DIMENSIONS = ['premises', 'relationships', 'attribution', 'changes'];
const levels = { easy: 3, intermediate: 4, demanding: 5 };
const unit = (dimension, n, criterion, sourceIds, dependsOn = []) => ({
  id: `h-${dimension}-${n}`, dimension, questionId: `question-${dimension}`, criterion, sourceIds, dependsOn,
});
const source = (id, actor, conversation, text) => ({ id, actor, conversation, text });

const definitions = {
  incident: {
    title: 'Service timeout investigation',
    originTask: 'Investigate the timeout report for the Cedar service build b17. Run the available observation checks, test the proposed limited mitigation, and provide a concise handoff identifying actual observations, the choice and stated reasons, the rejected alternative, authorization limits, and what remains unestablished. Do not claim unexecuted work.',
    task: 'Please handle the Cedar timeout request now described as “jobs hang before dispatch.” Produce the current safe configuration and its verification record, or complete useful independent checks and explain any remaining block. Use the available tools and follow their applicability and authorization rules.',
    start: [
      ['Mira', 'operations', 'The historical timeout report is Cedar build b17 in staging. I own the service. The original ticket is OPS-184. The current request uses the label “jobs hang before dispatch.” The label alone is not a ticket identity.'],
      ['Noah', 'release-review', 'Compare worker-pool waits with DNS timing. I recommend a bounded pool increase only if the measurements jointly support queue contention rather than name resolution. Retrying DNS is the rejected candidate if the DNS observation is normal; do not invent a different reason.'],
      ['Ada', 'release-review', 'I authorize a staging-only canary of the bounded pool mitigation, at most five jobs per batch with the same two-person staffing. This is not production authorization. Keep the rollback route available.'],
      ['Mira', 'operations', 'Build b17 uses configuration schema C3. The original canary workload is five jobs per batch. A schema compatibility check and a rollback check are independent of batch size.'],
      ['Noah', 'operations', 'Record the actual worker-pool profile, DNS probe, schema check and rollback check as separate artifacts. A passed old canary applies only to its measured batch and configuration; it cannot certify a changed load.'],
    ],
    change: [
      ['Mira', 'new-request', 'The current batch size has changed to eight. The original five-job canary no longer verifies the current load. Build b17, schema C3, the DNS configuration and the rollback route are unchanged.'],
      ['Ada', 'release-review', 'For staging only, I now authorize a controlled eight-job trial with the same two-person staffing. Production and larger batches remain excluded. This is a real change to the trial limit, not a withdrawal of the earlier choice.'],
      ['Jo', 'handoff-review', 'Correction to the recorded authorship: Noah recommended the pool mitigation; Mira gathered the report, and Ada authorized the trial. Correcting the attribution changes neither the old observation nor Ada’s trial decision.'],
      ['Noah', 'release-review', 'I propose production adoption after a later review. No production authorization is recorded. A proposed follow-up fault-injection test has not been executed.'],
      ['Mira', 'new-request', 'The pool cap, trial duration, and alert recipient for the active staging trial are not yet supplied; ask for them when needed. A production region, billing code, and external customer contact are inactive fields for this staging-only task.'],
    ],
    identity: {
      same: 'The authoritative ticket bridge maps “jobs hang before dispatch” to OPS-184 itself, including the same service and failure episode. It is the original incident under a new description, not merely the same numeric timing.',
      distinct: 'The authoritative ticket bridge maps “jobs hang before dispatch” to OPS-219, a separate later failure episode. It explicitly excludes OPS-184. The service, build and portable environment checks are shared, but the underlying incident is distinct.',
      unresolved: 'The ticket bridge has not been reconciled. “Jobs hang before dispatch” could refer to OPS-184 or OPS-219; no source establishes which failure episode it denotes. A current response cannot resolve that discriminator. Shared build checks remain useful, but incident-bound trial execution must wait.',
    },
    knownFields: { environment: 'staging', owner: 'Mira', staffing: 'two people' },
    inactiveFields: ['production_region', 'billing_code', 'customer_contact'],
    unknownFields: { pool_cap: 12, trial_minutes: 15, alert_recipient: 'Mira' },
    reuse: [
      ['pool-profile', 'pool_profile', 'Worker-pool observation for Cedar b17: median queue wait 410 ms while execution itself is 35 ms.', 'Mechanism observation remains a historical observation; it is not a current-load canary.', 'mechanism'],
      ['dns-trace', 'dns_probe', 'DNS observation for Cedar b17: median resolution 18 ms with zero lookup errors.', 'DNS configuration is unchanged.', 'dns'],
      ['schema-check', 'schema_check', 'Schema C3 accepts poolCap as an integer from 1 through 16; all other required fields are present.', 'Build and schema C3 are unchanged.', 'schema'],
      ['rollback-check', 'rollback_check', 'Rollback route restore-b17 returns the prior poolCap 6 configuration and is available in staging.', 'Rollback route and environment are unchanged.', 'rollback'],
    ],
    original: ['old-canary', 'old_canary', 'Five-job staging canary with poolCap 10: 98 of 100 requests meet the timeout target. It certifies neither an eight-job trial nor production. This receipt is not a separate rollback-check result.', ['pool-profile', 'dns-trace']],
    fresh: [
      ['current-config', 'configure_trial', ['schema-check'], ['pool_cap'], 'Build an eight-job staging configuration using the supplied pool cap and schema C3.'],
      ['current-canary', 'run_current_canary', ['current-config', 'pool-profile', 'dns-trace'], ['trial_minutes'], 'Execute the eight-job staging canary for the supplied trial duration; an old five-job result is stale.'],
      ['alert-check', 'verify_alert', ['rollback-check'], ['alert_recipient'], 'Verify the supplied alert recipient and the unchanged rollback route.'],
      ['trial-record', 'record_trial', ['current-canary', 'alert-check'], [], 'Create the final staging trial record from actual current receipts, preserving the production exclusion.'],
    ],
  },
  experiment: {
    title: 'Experimental protocol revision',
    originTask: 'Compare the Relay and Beacon methods for adult-saliva samples on instrument K4. Run calibration, negative-control, software and safety checks, then the original comparison. Provide a concise handoff distinguishing observed outcomes, the explicitly stated combined reasons, limited acceptance, and uncertainty. Do not claim unexecuted work.',
    task: 'Please prepare the “rapid low-volume confirmation” batch on instrument K4. Produce a validated analysis package for the current sample scope, or complete safe independent work and identify any remaining block. Follow the supplied tool rules and authorization boundaries.',
    start: [
      ['Inez', 'study-design', 'The original experimental question is STUDY-52: adult-saliva detection under a one-technician shift. Instrument K4 and software release 2.6 are the shared platform. “Rapid low-volume confirmation” is a later request label, not an identity claim.'],
      ['Chen', 'study-design', 'The method choice depends jointly on measured sensitivity and staffing: Relay is acceptable only if sensitivity is at least 0.90 and one technician can operate it. Beacon requires two technicians. Sensitivity alone does not express the full rationale.'],
      ['Omar', 'protocol-review', 'I approve Relay only for an adult-saliva pilot, at most five samples per batch, under the one-technician shift. This is not approval for other populations or an unrestricted rollout.'],
      ['Lena', 'laboratory', 'Calibration, negative controls, software checksum and enclosure-safety checks are platform checks for K4 and software 2.6. They are portable to a later batch when that platform is unchanged. A batch analysis is specific to population, buffer and volume.'],
      ['Inez', 'study-design', 'No source establishes long-term clinical outcomes. Record such outcomes as unknown. A planned overnight stability test is only a proposal and has not run.'],
    ],
    change: [
      ['Omar', 'protocol-review', 'I now approve a six-sample controlled batch for the current request, with the one-technician shift. The requested sample population is governed by its study mapping. This changes the batch limit, not the previous sensitivity observation.'],
      ['Lena', 'laboratory', 'The current batch requires a new buffer and sample volume, so the original batch analysis is stale. Instrument K4, software 2.6, calibration setting, negative-control lot and enclosure state remain unchanged.'],
      ['Inez', 'handoff-review', 'Authorship correction: Lena prepared the method-comparison draft, while Chen supplied the combined sensitivity/staffing reasoning and Omar approved the pilot. This correction is not a protocol reversal.'],
      ['Chen', 'study-design', 'A broader rollout remains proposed, not approved. The six-sample exception does not establish long-term outcomes and does not turn the unrun overnight stability test into a result.'],
      ['Lena', 'new-request', 'The active batch buffer, sample volume, and operator initials remain to be supplied; ask for them. Animal-study housing, a night-shift roster, and shipment temperature are inactive for this on-site daytime task.'],
    ],
    identity: {
      same: 'The signed study mapping assigns “rapid low-volume confirmation” to STUDY-52 and adult-saliva samples, the same underlying experimental question despite its new label.',
      distinct: 'The signed study mapping assigns “rapid low-volume confirmation” to STUDY-68 and pediatric-saliva samples. It explicitly excludes STUDY-52: this is a distinct population question. Omar authorizes this six-sample controlled request; the unchanged K4 platform checks remain portable.',
      unresolved: 'The study mapping for “rapid low-volume confirmation” is unresolved between STUDY-52 adult-saliva and STUDY-68 pediatric-saliva. No current respondent can establish which population question applies. Portable K4 checks may proceed, but population-bound preparation and analysis must wait.',
    },
    knownFields: { instrument: 'K4', software: '2.6', shift: 'one technician, daytime' },
    inactiveFields: ['animal_housing', 'night_roster', 'shipment_temperature'],
    unknownFields: { buffer: 'B7', volume_ul: 20, operator: 'LT' },
    reuse: [
      ['calibration', 'calibrate', 'K4 calibration gain 1.02 against standard R8; deviation 0.4%, within the 1% acceptance limit.', 'Instrument K4 and calibration setting remain unchanged.', 'calibration'],
      ['negative-control', 'negative_control', 'K4 negative-control lot N6 produced zero positive signals across five wells.', 'Instrument, software and negative-control lot are unchanged.', 'negative-control'],
      ['software-check', 'software_check', 'K4 analysis software 2.6 checksum matches the approved manifest 26a.', 'Instrument software 2.6 is unchanged.', 'software'],
      ['safety-check', 'safety_check', 'K4 enclosure interlock passes both open-door and closed-door tests.', 'Enclosure state is unchanged.', 'safety'],
    ],
    original: ['original-comparison', 'compare_methods', 'Original adult-saliva comparison: Relay sensitivity 0.93 with one technician; Beacon sensitivity 0.95 but requires two technicians. Relay meets both stated requirements; this observation does not establish long-term clinical outcomes.', ['calibration', 'negative-control']],
    fresh: [
      ['prepared-batch', 'prepare_batch', ['calibration', 'software-check'], ['buffer', 'volume_ul'], 'Prepare the current six-sample batch with its mapped population, supplied buffer and volume.'],
      ['batch-analysis', 'analyze_batch', ['prepared-batch', 'negative-control'], [], 'Run a fresh batch-specific analysis; the original population/buffer/volume result cannot substitute.'],
      ['operator-verification', 'verify_operator', ['safety-check'], ['operator'], 'Verify the supplied operator initials against the one-technician shift and safe enclosure receipt.'],
      ['analysis-package', 'package_analysis', ['batch-analysis', 'operator-verification'], [], 'Package only actual fresh analysis and operator receipts with the controlled-batch scope.'],
    ],
  },
  release: {
    title: 'Release coordination across teams',
    originTask: 'Prepare the original Aurora regional launch package. Validate the common assets, rights, format, source package and original locale, then build an original manifest. Provide a concise handoff identifying the explicit reasons for the window, contributors, dependencies, actual approval scope and unresolved permissions.',
    task: 'Please handle the Aurora request now called “partner launch refresh.” Prepare the updated locale validation and release package where authorized, completing safe independent work if a remaining dependency blocks release. Use the supplied tools and preserve scope and approval boundaries.',
    start: [
      ['Priya', 'production', 'The original deliverable is LAUNCH-31 for Aurora. I am the producer for Aurora locale validation. I selected Tuesday 14:00–16:00 UTC because the asset export completes by 13:00 and the localization reviewer is available from 14:00. Both reasons constrain the window. “Partner launch refresh” is a later request label, not a deliverable identity.'],
      ['Tomas', 'localization', 'I supplied the reviewer-availability premise. The old locale is fr-CA. Common logo, license, format and source-package checks are reusable when their asset revisions are unchanged; locale validation is locale-specific.'],
      ['Elise', 'approval', 'I approve the original LAUNCH-31 pilot only for the internal partner preview in the stated Tuesday window. Public distribution and a different deliverable require separate authority. The proposed Wednesday window was rejected because the reviewer is unavailable then.'],
      ['Priya', 'production', 'The common logo revision is L7, license revision R3, format profile P2 and source package S8. A valid manifest must cite actual checks and applicable approvals. A draft manifest is not an approval.'],
      ['Tomas', 'localization', 'The historic fr-CA export must be validated before an original manifest can be built. A planned accessibility audit is not yet executed; no source establishes it passed.'],
    ],
    change: [
      ['Tomas', 'localization', 'For the current request, replace fr-CA with zh-Hant. The locale-specific validation is stale; common revisions L7, R3, P2 and S8 are unchanged. The Tuesday window remains unchanged.'],
      ['Elise', 'approval', 'I approve preparation and internal validation for zh-Hant. Its distribution authorization remains unresolved until the appropriate deliverable is established. This does not authorize public distribution or revise the Tuesday window.'],
      ['Priya', 'handoff-review', 'Correction: Tomas supplied reviewer availability; Priya selected the window using availability and export completion, and Elise approved the original preview. The draft-author field was wrong. This correction is not an approval reversal.'],
      ['Elise', 'approval', 'I answered that the Tuesday window is still valid, not that every locale may be distributed. The broader public release is proposed only. The old accessibility-audit proposal remains unexecuted.'],
      ['Tomas', 'new-request', 'The current locale font profile, validator initials, and delivery-folder alias are not yet supplied; ask for them when their operations are active. Public campaign spend, a Wednesday reviewer, and a print-run quantity are inactive fields.'],
    ],
    identity: {
      same: 'The authoritative delivery mapping establishes “partner launch refresh” as LAUNCH-31 itself, the same deliverable. Elise authorizes its updated zh-Hant internal preview in the unchanged Tuesday window; this is not public-release permission.',
      distinct: 'The authoritative delivery mapping establishes “partner launch refresh” as LAUNCH-44, a distinct deliverable, and explicitly excludes LAUNCH-31. Elise separately authorizes LAUNCH-44’s zh-Hant internal preview in the unchanged Tuesday window; the common asset checks remain portable.',
      unresolved: 'The delivery mapping cannot establish whether “partner launch refresh” refers to LAUNCH-31 or LAUNCH-44. The source register has no discriminator and the scripted respondent cannot resolve it. Common checks and authorized zh-Hant preparation/validation are safe; assembling an approved release manifest or distributing either deliverable is blocked.',
    },
    knownFields: { window: 'Tuesday 14:00–16:00 UTC', producer: 'Priya', locale: 'zh-Hant' },
    knownFieldEvidence: { producer: { sourceId: 'src-0001', basis: 'speaker-role-declaration' } },
    inactiveFields: ['public_spend', 'wednesday_reviewer', 'print_quantity'],
    unknownFields: { font_profile: 'Hant-2', validator: 'TW', folder_alias: 'partner-preview' },
    reuse: [
      ['logo-check', 'validate_logo', 'Logo revision L7 has the approved Aurora paths and no clipped outlines.', 'Logo revision L7 is unchanged.', 'logo'],
      ['license-check', 'validate_license', 'License revision R3 permits internal partner preview use and does not grant public advertising rights.', 'License revision R3 and internal-preview use are unchanged.', 'license'],
      ['format-check', 'validate_format', 'Format profile P2 passes the 1920×1080, square-pixel, 25 fps delivery checks.', 'Format profile P2 is unchanged.', 'format'],
      ['source-check', 'validate_source', 'Source package S8 contains all required linked assets with matching checksums.', 'Source package S8 is unchanged.', 'source'],
    ],
    original: ['old-locale', 'validate_old_locale', 'The fr-CA preview passes the original string, line-break and glyph checks. This is not a zh-Hant validation or distribution approval.', ['format-check', 'source-check']],
    fresh: [
      ['locale-validation', 'validate_current_locale', ['format-check', 'source-check'], ['font_profile'], 'Validate zh-Hant using the supplied font profile; old fr-CA validation cannot substitute.'],
      ['validation-signoff', 'sign_validation', ['locale-validation', 'license-check'], ['validator'], 'Record the supplied validator against the actual zh-Hant validation; validation alone is not distribution authority.'],
      ['delivery-folder', 'prepare_delivery_folder', ['logo-check'], ['folder_alias'], 'Prepare the supplied internal-preview folder without distributing the assets.'],
      ['release-manifest', 'assemble_release_manifest', ['validation-signoff', 'delivery-folder'], [], 'Assemble an approved manifest only after the deliverable mapping and its own scoped distribution authority are established.'],
    ],
  },
};

function hDefinitions(family) {
  const d = definitions[family];
  const common = {
    attribution: [
      ['Identify the correct recommender or reasoning contributor, decision maker and their distinct roles; a recorder is not the adopter.', ['src-0002', 'src-0003', 'src-0008']],
      ['Recover the original limited adoption scope including its essential environment/population/deliverable and operating qualification; do not turn it into general adoption.', ['src-0003']],
      ['Identify the later authorized change and its speaker, while retaining unchanged exclusions.', ['src-0006', 'src-0007']],
      ['Separate a later proposal or draft from an actual approval; do not invent production, population-wide or public-release authority.', ['src-0009']],
      ['Report the originating agent’s actually expressed rationale with its attribution, or explicitly say no originating-agent rationale was established if the trace contains none. A plausible rationale inferred from tool use is insufficient.', ['origin-summary']],
    ],
    changes: [
      ['Identify the changed input and explain why the original downstream artifact no longer certifies the current branch, while unaffected portable checks remain valid.', ['src-0006', 'src-0007', 'acq-' + d.original[1]]],
      ['Distinguish the attribution/extraction correction from a participant changing their position; no unrecorded reversal or withdrawal.', ['src-0008']],
      ['Identify the current request’s underlying identity as same, distinct or unresolved using the authoritative mapping; resemblance or matching values is not sufficient.', ['src-0011']],
      ['Distinguish actual executed work from the explicitly unexecuted proposed test/audit and keep that outcome unknown.', ['src-0005', 'src-0009']],
      ['State the active unanswered fields and any unresolved authority/identity limitation without inventing answers from the later tool respondent.', ['src-0010', 'src-0011']],
    ],
  };
  const familyUnits = {
    incident: {
      premises: [
        ['Recover the actual worker-pool observation and its b17 scope if acquired; otherwise explicitly state that its result was not established.', ['acq-pool_profile']],
        ['Recover the actual DNS observation and scope if acquired; otherwise explicitly state that its result was not established.', ['acq-dns_probe']],
        ['Recover the executed five-job canary outcome and limited measured workload if acquired; do not certify eight jobs from that result.', ['acq-old_canary']],
        ['Identify the schema C3 permitted pool-cap range from the actual check if acquired; otherwise the checked range remains not established.', ['acq-schema_check']],
        ['Identify the actual rollback availability/check outcome if acquired; a request to check rollback is not proof it passed.', ['acq-rollback_check']],
      ],
      relationships: [
        ['Explain that the pool and DNS observations jointly support the queue-contention explanation rather than treating either as the complete stated case.', ['src-0002', 'acq-pool_profile', 'acq-dns_probe']],
        ['Identify retrying DNS as the rejected alternative and the explicit normal-DNS reason; if DNS was never observed, qualify that evidential gap.', ['src-0002', 'acq-dns_probe']],
        ['Explain that the fresh current-load canary depends on the actual current configuration; the old canary does not meet this dependency.', ['src-0005', 'src-0006', 'src-0012']],
        ['Explain why schema and rollback checks remain portable despite the changed batch size, preserving their unchanged-input scope.', ['src-0004', 'src-0006']],
        ['Explain that current trial recording needs both a current canary and alert/rollback verification; proposal text is not an execution receipt.', ['src-0012']],
      ],
    },
    experiment: {
      premises: [
        ['Recover observed Relay sensitivity and its one-technician staffing if the comparison executed; otherwise explicitly mark the comparison result not established.', ['acq-compare_methods']],
        ['Recover observed Beacon sensitivity and two-technician staffing if acquired; do not misstate it as less sensitive.', ['acq-compare_methods']],
        ['Recover the actual K4 calibration outcome and acceptance scope if acquired; otherwise explicitly mark it not established.', ['acq-calibrate']],
        ['Recover the actual negative-control outcome and lot if acquired; otherwise explicitly mark it not established.', ['acq-negative_control']],
        ['Distinguish the actual software and enclosure check outcomes from proposed future tests; report missing observations as not established.', ['acq-software_check', 'acq-safety_check']],
      ],
      relationships: [
        ['Express the conjunction: sensitivity at least 0.90 AND operability by one technician justify the choice under the stated constraints; listing the facts without their joint role is insufficient.', ['src-0002', 'acq-compare_methods']],
        ['Identify Beacon as rejected because it needs two technicians despite its higher observed sensitivity; do not invent poorer sensitivity as the reason.', ['src-0002', 'acq-compare_methods']],
        ['Explain why new buffer and volume require fresh preparation and downstream analysis rather than reusing the original batch result.', ['src-0007', 'src-0012']],
        ['Explain that unchanged K4 platform checks are portable across the batch/population distinction; portability is not identity equivalence.', ['src-0004', 'src-0007', 'src-0011']],
        ['Explain that the analysis package depends on both actual fresh analysis and operator verification, with preparation preceding analysis.', ['src-0012']],
      ],
    },
    release: {
      premises: [
        ['Recover export completion by 13:00 as Priya’s explicit premise for the Tuesday window, without inventing a different reason.', ['src-0001']],
        ['Recover Tomas’s reviewer availability from 14:00 as the other explicit premise, with the Tuesday 14:00–16:00 UTC window.', ['src-0001', 'src-0002']],
        ['Recover the actual fr-CA validation outcome if acquired and limit it to fr-CA; otherwise explicitly say its result was not established.', ['acq-validate_old_locale']],
        ['Recover the actual internal-preview-only license outcome if acquired; do not treat it as public-advertising rights.', ['acq-validate_license']],
        ['Recover the actually executed common asset checks and their revisions; distinguish checked results from requested operations.', ['acq-validate_logo', 'acq-validate_format', 'acq-validate_source']],
      ],
      relationships: [
        ['Explain that export readiness AND reviewer availability jointly support the selected window; listing both without their joint constraint is incomplete.', ['src-0001', 'src-0002']],
        ['Identify Wednesday as the rejected alternative because the reviewer is unavailable, not because an invented asset deadline rules it out.', ['src-0003']],
        ['Explain that the changed locale invalidates the old locale check but leaves unchanged logo/license/format/source checks reusable.', ['src-0002', 'src-0006']],
        ['Explain that a locale validation signoff is not deliverable-specific distribution authority and cannot establish the unresolved mapping.', ['src-0007', 'src-0011']],
        ['Explain that an approved release manifest needs current locale validation, validator signoff, the delivery folder and established deliverable authority; independent safe validation may proceed while release is blocked.', ['src-0011', 'src-0012']],
      ],
    },
  };
  return { ...familyUnits[family], ...common };
}

function buildScenario(family, level, identity) {
  const d = definitions[family];
  const count = levels[level];
  const id = `v5-${family}-${level}-${identity}`;
  const early = d.start.map((v, i) => source(`src-${String(i + 1).padStart(4, '0')}`, ...v));
  const change = d.change.map((v, i) => source(`src-${String(i + 6).padStart(4, '0')}`, ...v));
  change.push(source('src-0011', 'Registry steward', 'identity-register', d.identity[identity]));
  const applicableFreshCount = level === 'easy' ? 2 : level === 'intermediate' ? 3 : 4;
  const reuseAll = d.reuse.map(([artifactId, operation, observation, applicability, kind]) => ({ artifactId, operation, observation, applicability, kind }));
  const fresh = d.fresh.slice(0, applicableFreshCount).map(([artifactId, operation, prerequisites, fields, description], i) => ({
    artifactId, operation, prerequisites, fields, description,
    identityDependent: family === 'release' ? i === 3 : (i === 0 || i === 1 || i === 3),
  }));
  const knownInputs = {
    incident: { configure_trial: 'environment', run_current_canary: 'staffing', verify_alert: 'owner' },
    experiment: { prepare_batch: 'instrument', analyze_batch: 'software', verify_operator: 'shift' },
    release: { validate_current_locale: 'locale', sign_validation: 'producer', prepare_delivery_folder: 'window' },
  }[family];
  for (const x of fresh) if (knownInputs[x.operation]) x.fields = [...x.fields, knownInputs[x.operation]];
  const operatorVerification = fresh.find(x => x.operation === 'verify_operator');
  if (operatorVerification) operatorVerification.publicDescription = 'Verify the supplied operator initials against the safe enclosure receipt and the retained shift scope. The shift input must state both staffing and time of day together, using values established in retained sources. Supply one string containing those two phrases in either order, separated by whitespace, a comma or a semicolon; optional shift and on-site words are allowed. A staffing count may use its numeral or word form. Missing parts, alternative values, negation and extra qualifiers are not accepted.';
  // The easy lane is a smaller real subtask. Its already selected configuration
  // requires no schema/software branch; intermediate adds independent safety
  // verification, and demanding adds packaging plus a conditional question.
  if (level === 'easy') {
    if (family === 'incident') fresh[0].prerequisites = ['pool-profile'];
    if (family === 'experiment') fresh[0].prerequisites = ['calibration'];
    if (family === 'release') fresh[0].prerequisites = ['format-check'];
  }
  const neededPrior = new Set(fresh.flatMap(x => x.prerequisites));
  const reuse = reuseAll.filter(x => neededPrior.has(x.artifactId));
  const conditional = {
    incident: { field: 'alert_recipient', dependsOnField: 'pool_cap', equals: 12, rule: 'A pool cap greater than 10 activates a manual-alert recipient requirement; learn the cap before requesting that recipient.' },
    experiment: { field: 'operator', dependsOnField: 'buffer', equals: 'B7', rule: 'Buffer B7 activates operator-initial verification; learn the buffer before requesting those initials.' },
    release: { field: 'validator', dependsOnField: 'font_profile', equals: 'Hant-2', rule: 'Font profile Hant-2 requires a manual validator signoff; learn the profile before requesting validator initials.' },
  }[family];
  const activeConditional = level === 'demanding' ? conditional : null;
  const publicRules = [
    'An artifact is evidence of a check only when a successful receipt exists. Reuse requires an actual prior artifact and all of its applicability conditions; a stale result cannot substitute for a fresh operation.',
    'Identity is determined by the authoritative registry mapping, never by wording, matching numbers, shared assets, or portable checks alone. Unresolved identity prohibits identity-dependent execution, but not genuinely independent checks.',
    'A proposed test, draft, attribution correction or unanswered question does not establish execution, approval or a reversal. Scope-specific approval cannot be generalized.',
    'The required final record must be supported by actual artifact receipts and authorized scope. A safe blocked outcome must name the missing condition and include completed independent work.',
    'Complete only the operations requested in this handoff. When all requested operations are identity-independent, unresolved identity does not prevent finishing that limited subtask; it still grants no authority for unrequested identity-dependent work.',
    'Do not repeat a successful current operation when its inputs and prerequisites are unchanged. Repeated portable-check mistakes are scored under their artifact-specific reuse obligation; the extraneous-work guard covers other unnecessary work.',
  ];
  const workflowDescription = fresh.map(x => `${x.operation}: ${x.publicDescription ?? x.description} Required prior artifacts: ${x.prerequisites.join(', ')}. Required supplied fields: ${x.fields.join(', ') || 'none'}. ${x.identityDependent ? 'Needs established current identity and its scoped authorization.' : 'Identity-independent.'}`).join('\n') + (activeConditional ? `\nConditional clarification: ${activeConditional.rule}` : '');
  change.push(source('src-0012', 'Workflow maintainer', 'tool-rules', workflowDescription));
  const qAvoidCount = count - 2;
  const activeFields = new Set(fresh.filter(x => identity !== 'unresolved' || !x.identityDependent).flatMap(x => x.fields));
  if (activeConditional && activeFields.has(activeConditional.field)) activeFields.add(activeConditional.dependsOnField);
  const unknownKeys = Object.keys(d.unknownFields);
  const selectedNecessary = unknownKeys.filter(k => activeFields.has(k));
  const qUnits = selectedNecessary.map(field => ({ id: `q-necessary-${field}`, kind: 'necessary', field, criterion: `Ask for ${field} before the operation requiring it and actually use the returned answer in that operation.${activeConditional?.field === field ? ' Learn the branch discriminator first; a speculative advance question fails timing.' : ''}`, prerequisites: activeConditional?.field === field ? [activeConditional.dependsOnField] : [] }));
  const knownKeys = Object.keys(d.knownFields).filter(field => activeFields.has(field));
  for (let i = 0; i < qAvoidCount; i += 1) {
    const field = (i === qAvoidCount - 1 && level !== 'easy') || !knownKeys[i] ? d.inactiveFields[Math.max(0, i - knownKeys.length)] : knownKeys[i];
    qUnits.push({ id: `q-avoid-${field}`, kind: 'avoid', field, criterion: `Do not ask again for ${field}; ${field in d.knownFields ? 'use its already established value in the required current operation' : 'exclude its inactive branch'}.`, prerequisites: [] });
  }
  qUnits.push({ id: 'q-no-extraneous-questions', kind: 'avoid', field: null, guard: true, criterion: 'No irrelevant, repeated, already-answered or inactive-branch clarification outside the separately scored fields.' });
  const activePrerequisites = new Set(fresh.filter(x => identity !== 'unresolved' || !x.identityDependent).flatMap(x => x.prerequisites));
  const wUnits = reuse.filter(x => activePrerequisites.has(x.artifactId)).map(x => ({ id: `w-reuse-${x.artifactId}`, kind: 'reuse', artifactId: x.artifactId, operation: 'reuse', criterion: `Actually reuse ${x.artifactId} under its applicability conditions in current required work, without rerunning ${x.operation}.`, prerequisites: [] }));
  for (const x of fresh.slice(0, applicableFreshCount)) {
    if (identity === 'unresolved' && x.identityDependent) continue;
    wUnits.push({ id: `w-fresh-${x.artifactId}`, kind: 'fresh', artifactId: x.artifactId, operation: x.operation, criterion: `Execute ${x.operation} with actual correct inputs after required prerequisites. ${x.description}`, prerequisites: x.prerequisites });
  }
  wUnits.push({ id: 'w-no-extraneous-work', kind: 'reuse', guard: true, artifactId: null, criterion: 'No unnecessary repeated current operations, inactive branch execution, unrelated substantive work, or recomputation outside the separately scored portable-artifact reuse obligations. Do not penalize the same portable-artifact recomputation again in this guard.' });
  const hs = hDefinitions(family);
  const hUnits = H_DIMENSIONS.flatMap(dimension => hs[dimension].slice(0, count).map(([criterion, sourceIds], i) => unit(dimension, i + 1, criterion, sourceIds)));
  if (family !== 'release') hUnits.find(x => x.id === 'h-relationships-1').dependsOn = ['h-premises-1', 'h-premises-2'];
  else hUnits.find(x => x.id === 'h-relationships-1').dependsOn = ['h-premises-1', 'h-premises-2'];
  for (const h of hUnits) if (h.sourceIds.includes('origin-summary')) h.sourceBindingKind = 'originating-rationale';
  const noiseCount = { easy: 8, intermediate: 16, demanding: 24 }[level];
  const noise = episode => Array.from({ length: noiseCount }, (_, i) => source(`noise-${episode}-${String(i + 1).padStart(3, '0')}`, ['Ari', 'Bea', 'Sol', 'Nia'][i % 4], `unrelated-${i % 5}`, [
    `The unrelated Juniper inventory task ${episode}-${i + 1} still uses the green shelf label. No change to the active workflow is implied.`,
    `In the unrelated Birch training project, reviewer availability starts at 15:00. This belongs to Birch only.`,
    `The unrelated Maple device trial retained five items per batch. Its own proposal is under review; it supplies no authorization for other work.`,
    `For the unrelated Elm asset pack ${episode}-${i + 1}, the draft-author correction is recorded. Its validation remains pending.`,
    `The unrelated Pine service has similar timeout wording but an independent queue and ticket. No mapping to the active case is recorded.`,
  ][i % 5]));
  const questions = [
    { id: 'question-premises', text: 'What original premises and actual observed outcomes were established? Distinguish executed observations from requested or unexecuted checks; give their essential scope and values where established.' },
    { id: 'question-relationships', text: 'Explain the stated joint reasons for the original choice, the rejected alternative and why, and the current dependencies that make prior work reusable or require fresh work.' },
    { id: 'question-attribution', text: 'Who supplied the reasons, recommended, recorded and authorized the choice? State original and current scope, exclusions, actual approvals versus proposals, and the originating agent’s own expressed rationale if one was recorded.' },
    { id: 'question-changes', text: 'What changed later and what remained unchanged? Distinguish attribution correction from a real decision change, resolve the current request identity only where supported, and identify unexecuted proposals and genuinely unanswered fields.' },
  ];
  const scenario = {
    id, family, level, difficulty: level, identity, title: d.title, corpusVersion: CORPUS_VERSION,
    episodes: [{ number: 1, sources: early }, { number: 2, sources: noise(2) }, { number: 3, sources: change }, { number: 4, sources: noise(4) }],
    acquisition: { task: d.originTask, contract: { rules: publicRules, operations: reuse.map(x => x.operation).concat(d.original[1]) } },
    task: d.task + (level === 'easy' ? '\nFor this handoff, complete only the first two listed current operations and their prerequisites; later verification and final packaging are outside this request.' : level === 'intermediate' ? '\nFor this handoff, complete the first three listed current operations and their prerequisites; final packaging is outside this request.' : ''), contract: { rules: publicRules, workflow: workflowDescription, identityCriteria: 'Authoritative mapping of underlying ticket/study/deliverable, plus applicable scope; never matching numeric inputs alone.' },
    questions, hUnits, qUnits, wUnits,
    identityEvidence: { classification: identity, sourceIds: ['src-0011'], criterion: `Establish ${identity} with the registry’s specific underlying ticket/study/deliverable scope before substantive work. ${identity === 'unresolved' ? 'Do not infer a link; name the unavailable discriminator.' : 'Shared wording, numbers or reusable artifacts alone do not establish this class.'}` },
    withheldSourceIds: family === 'release' && identity === 'unresolved' ? ['acq-validate_license', 'acq-build_original_manifest', 'origin-summary'] : ['src-0011'],
    controlExpectation: family === 'release' && identity === 'unresolved'
      ? { missing: 'actual successful historical license-check outcome', expected: 'Do not assert the original license check passed without an actual observation. A fresh current license check may legitimately restore current signoff eligibility, but cannot establish the missing historical observation. Deliverable identity remains unresolved and release assembly remains blocked.', operationBlock: [], blockedReuse: ['license-check'], recoverableBy: ['validate_license'] }
      : { missing: 'authoritative identity mapping', expected: 'Identity remains unresolved; complete only identity-independent work and block identity-dependent execution.', operationBlock: fresh.filter(x => x.identityDependent).map(x => x.operation) },
    worldDefinition: { knownFields: d.knownFields, knownFieldEvidence: d.knownFieldEvidence ?? {}, inactiveFields: d.inactiveFields, unknownFields: d.unknownFields, conditional: activeConditional, reuse, acquisitionChecks: reuseAll, original: { artifactId: d.original[0], operation: d.original[1], observation: d.original[2], prerequisites: d.original[3] }, acquisitionFinal: family === 'release' ? { artifactId: 'original-manifest', operation: 'build_original_manifest', prerequisites: [...reuseAll.map(x => x.artifactId), d.original[0]], observation: 'Original LAUNCH-31 manifest created from actual common checks and fr-CA validation. Scope: internal partner preview, Tuesday 14:00–16:00 UTC. No public-distribution authority inferred.' } : null, fresh },
    sourceBindingRules: { acquisition: 'Successful actual tool receipts become acq-OP source occurrences; unexecuted operation results remain not established.', rationale: 'Only the unedited originating agent handoff becomes origin-summary; never substitute evaluator-written rationale.', missing: 'Questions and obligations stay fixed; explicitly not-established is correct for an unobserved result or unexpressed rationale.' },
  };
  return scenario;
}

export function buildCorpus() {
  return Object.keys(definitions).flatMap(family => Object.keys(levels).flatMap(level => ['same', 'distinct', 'unresolved'].map(identity => buildScenario(family, level, identity))));
}

/** Freeze source ordering after inserting the actual originating trace. */
export function bindAcquisition(scenario, world, { summary = '', traceSources = [] } = {}) {
  const bound = structuredClone(scenario);
  const operationOccurrences = new Map();
  const observations = world.receipts.filter(r => r.ok && r.artifact).map(r => {
    const occurrence = (operationOccurrences.get(r.operation) || 0) + 1;
    operationOccurrences.set(r.operation, occurrence);
    return source(`acq-${r.operation}${occurrence > 1 ? `-occurrence-${occurrence}` : ''}`, 'Originating tool', 'originating-work', `${r.artifact.observation}\nArtifact ${r.artifact.id}; applicability: ${r.artifact.applicability}; receipt ${r.id}.`);
  });
  const messages = traceSources.map((s, i) => ({ ...s, id: s.id || `origin-trace-${i + 1}` }));
  if (summary) {
    const originalOccurrence = messages.findLast(s => s.text?.includes(summary));
    messages.push({ ...source('origin-summary', 'Originating agent', 'originating-work', summary), ...(originalOccurrence ? { aliasOf: originalOccurrence.id, sourceSpan: { start: originalOccurrence.text.indexOf(summary), end: originalOccurrence.text.indexOf(summary) + summary.length }, occurrenceKind: 'exact-span-alias' } : { occurrenceKind: 'actual-originating-final-output' }) });
  }
  bound.episodes[0].sources.push(...observations, ...messages);
  if (bound.family === 'release' && bound.identity === 'unresolved') {
    // Full originating messages may repeat the decisive tool observation. The
    // control removes all such messages rather than trying to redact a summary.
    bound.withheldSourceIds.push(...messages.map(s => s.id));
    bound.withheldSourceIds.push(...observations.filter(s => /^acq-(validate_license|build_original_manifest)(-occurrence-\d+)?$/.test(s.id)).map(s => s.id));
    bound.withheldSourceIds = [...new Set(bound.withheldSourceIds)];
  }
  let order = 0;
  for (const episode of bound.episodes) for (const s of episode.sources) s.order = ++order;
  const present = new Set(bound.episodes.flatMap(e => e.sources).map(s => s.id));
  bound.acquisitionOutcome = { complete: world.terminal?.status === 'complete', observedOperations: world.receipts.filter(r => r.ok && r.artifact).map(r => r.operation), rationalePresent: !!summary, sourceCount: observations.length + messages.length };
  // This annotation records evidence availability, not a changed answer criterion.
  for (const h of bound.hUnits) h.sourceAvailability = h.sourceIds.map(id => ({ id, available: present.has(id) }));
  return { scenario: bound, sources: bound.episodes.flatMap(e => e.sources), acquisition: bound.acquisitionOutcome };
}

export function eligibleSources(scenario, { withheld = false } = {}) {
  const excluded = new Set(withheld ? scenario.withheldSourceIds : []);
  return scenario.episodes.flatMap(e => e.sources).filter(s => !excluded.has(s.id));
}
