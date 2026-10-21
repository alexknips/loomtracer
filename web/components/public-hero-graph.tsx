import styles from "./public-hero-graph.module.css";

type HeroNodeProps = {
  id: string;
  kind: "Attempt" | "Evidence" | "Decision";
  title: string;
  summary: string;
  author: string;
  stage: string;
};

function HeroNode({ id, kind, title, summary, author, stage }: HeroNodeProps) {
  const candidate = kind === "Decision";
  const category = kind === "Attempt" ? "experiment" : kind === "Evidence" ? "finding" : "decision";
  return <article
    className={`${styles.node} ${styles[kind.toLowerCase() as "attempt" | "evidence" | "decision"]}`}
    aria-labelledby={`${id}-title`}
    data-hero-node={category}
    data-hero-status={candidate ? "candidate" : "observed"}
  >
    {kind !== "Attempt" && <span className={`${styles.port} ${styles.inputPort}`} aria-hidden="true" />}
    <div className={styles.nodeHeader}>
      <span className={styles.kindIcon} aria-hidden="true">{kind === "Attempt" ? "T" : kind === "Evidence" ? "E" : "D"}</span>
      <span className={styles.kind}>{kind}</span>
      <span className={`${styles.status} ${candidate ? styles.candidateStatus : ""}`}>{candidate ? "Candidate" : "Observed"}</span>
    </div>
    <div className={styles.project}>Collaboration app / Pricing</div>
    <h3 id={`${id}-title`}>{title}</h3>
    <p>{summary}</p>
    {candidate && <div className={styles.ratification}><span aria-hidden="true" />Awaiting human ratification</div>}
    <footer><span>{author}</span><span>{stage}</span></footer>
    {kind !== "Decision" && <span className={`${styles.port} ${styles.outputPort}`} aria-hidden="true" />}
  </article>;
}

function DocumentsConnection() {
  return <div className={styles.branchConnection} aria-label="The experiment documents its findings">
    <svg viewBox="0 0 40 34" preserveAspectRatio="none" aria-hidden="true"><path d="M20 0 V32 M16 27 L20 32 L24 27" /></svg>
    <span>documents</span>
  </div>;
}

export function PublicHeroGraph() {
  return <figure className={styles.frame} aria-label="Fictional pricing study graph" data-hero-graph>
    <figcaption className={styles.caption}><span>Fictional pricing study</span><span>Read-only sample</span></figcaption>
    <div className={styles.graph}>
      <div className={styles.branches}>
        <div className={styles.branch} role="group" aria-label="Agent A's per-seat investigation">
          <HeroNode id="hero-seat-experiment" kind="Attempt" title="Explore per-seat pricing" summary="Compare predictable spending with the cost of adding people." author="Agent A" stage="Exploration" />
          <DocumentsConnection />
          <HeroNode id="hero-seat-finding" kind="Evidence" title="Predictable bills, fewer invitations" summary="Seat charges can discourage occasional collaborators." author="Agent A" stage="Documented finding" />
        </div>
        <div className={styles.branch} role="group" aria-label="Agent B's usage investigation">
          <HeroNode id="hero-usage-experiment" kind="Attempt" title="Explore usage-based pricing" summary="Investigate a bill that scales with the team's activity." author="Agent B" stage="Exploration" />
          <DocumentsConnection />
          <HeroNode id="hero-usage-finding" kind="Evidence" title="Flexible usage, uncertain bills" summary="Activity-based bills can be harder to anticipate." author="Agent B" stage="Documented finding" />
        </div>
      </div>
      <div className={styles.convergence} aria-label="Both documented findings support the proposed pricing test">
        <svg className={styles.joinLines} viewBox="0 0 600 52" preserveAspectRatio="none" aria-hidden="true">
          <path d="M144 0 V16 Q144 24 152 24 H292 Q300 24 300 32 V49 M456 0 V16 Q456 24 448 24 H308 Q300 24 300 32 M296 44 L300 49 L304 44" />
        </svg>
        <span className={styles.leftRelation}>supports</span><span className={styles.rightRelation}>supports</span>
        <div className={styles.mobileJoin}><svg viewBox="0 0 40 46" preserveAspectRatio="none" aria-hidden="true"><path d="M20 0 V43 M16 38 L20 43 L24 38" /></svg><span>Both findings support</span></div>
      </div>
      <div className={styles.proposal}>
        <HeroNode id="hero-team-plan" kind="Decision" title="Test a team plan with included usage" summary="Draw on both findings and test a combined approach before choosing a price." author="Agent C" stage="Proposed test" />
      </div>
    </div>
    <p className={styles.disclosure}>Illustrative scenario and findings. No pricing decision has been approved.</p>
  </figure>;
}
