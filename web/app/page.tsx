import type { Metadata } from "next";
import Link from "next/link";
import { BrandLogo } from "@/components/brand-logo";
import { PublicHeader } from "@/components/public-header";
import { PublicHeroGraph } from "@/components/public-hero-graph";

const title = "Loomtracer — Test. Iterate. Connect the dots.";
const description = "Shared project memory that moves open questions to informed decisions. Connect your agents' investigations, findings, and choices so each agent can discover relevant history, explore solutions, and build on what works.";

export const metadata: Metadata = {
  title,
  description,
  robots: { index: true, follow: true },
  alternates: { canonical: "/" },
  openGraph: { title, description, url: "/", images: [{ url: "/og.png", width: 1731, height: 909, alt: "Loomtracer — Connect the dots." }] },
  twitter: { card: "summary_large_image", title, description, images: ["/og.png"] },
};

// The workspace is part of the full app; a public-only deployment serves this page and the demo alone.
const workspaceLink = process.env.NEUROFILAMENT_PUBLIC_ONLY !== "true";

export default function Home() {
  return <div className="public-site">
    <a className="public-skip" href="#public-main">Skip to content</a>
    <PublicHeader />
    <main id="public-main">
      <section className="public-hero public-container">
        <div className="public-hero-copy">
          <p className="public-eyebrow"><span /> For builders working with AI agents</p>
          <h1>Test. Iterate.<br /><em>Connect the dots.</em></h1>
          <p className="public-product-line">Shared project memory that moves open questions to informed decisions.</p>
          <p className="public-lede">Connect your agents&apos; investigations, findings, and choices–so each agent can discover relevant history, explore solutions, and build on what works.</p>
          <div className="public-actions"><Link className="public-button public-button-dark" href="/demo">Explore the demo <span aria-hidden="true">↗</span></Link>{workspaceLink && <Link className="public-text-link" href="/graph">Open the workspace <span aria-hidden="true">→</span></Link>}</div>
          <p className="public-hero-note">Open sample. No account needed.</p>
        </div>
        <PublicHeroGraph />
      </section>

      <section className="public-principle public-container" aria-labelledby="continuity-heading">
        <p className="public-eyebrow">Independent exploration. Shared progress.</p>
        <h2 id="continuity-heading">Stay agile. Branch out.<br />Keep your agents connected.</h2>
        <div className="public-feature-grid">
          <article><span className="public-step">01 / Possibilities</span><h3>Create space to explore.</h3><p>Keep meaningful alternatives visible while the evidence is still emerging. Promising paths deserve to be explored and tested before committing.</p></article>
          <article><span className="public-step">02 / Exploration</span><h3>Build on your findings.</h3><p>Findings from different agents can reinforce, challenge, or combine. Share your findings with your agents as they work.</p></article>
          <article><span className="public-step">03 / Direction</span><h3>Trace reasoning through changes.</h3><p>Once you make a decision, your agents understand what supports that decision and what might change it. This chain of logic creates a coherent record, even as your plan changes.</p></article>
        </div>
      </section>

      <section className="public-pricing-section" aria-labelledby="pricing-heading">
        <div className="public-container public-pricing-layout">
          <div className="public-pricing-intro">
            <div>
              <p className="public-eyebrow">Fictional pricing study</p>
              <h2 id="pricing-heading">One pricing question.<br />Three possible paths.</h2>
            </div>
            <div className="public-pricing-brief">
              <p>A collaboration app needs pricing that teams can predict without making them hesitate to invite people. Two agents investigate different models. A third picks up their findings and proposes the next experiment.</p>
              <p className="public-pricing-disclosure">The story behind the graph above. All experiments and findings in this example are fictional.</p>
            </div>
          </div>

          <div className="public-pricing-options" aria-label="Two pricing experiments and their documented findings">
            <article className="public-pricing-card public-pricing-study" aria-labelledby="seat-study-heading">
              <span className="public-mini-label">Agent A / Experiment 01</span>
              <h3 id="seat-study-heading">Explore per-seat pricing</h3>
              <dl>
                <div><dt>Question</dt><dd>Can a price per person keep the bill predictable without limiting collaboration?</dd></div>
                <div><dt>Experiment</dt><dd>A pricing prototype asks teams to add regular and occasional collaborators, then review how those invitations change the bill.</dd></div>
                <div className="public-pricing-observation"><dt>Documented finding</dt><dd><strong>Predictable bills, fewer invitations.</strong> A stable seat count is easy to budget for. An extra charge for each person makes occasional collaborators harder to justify.</dd></div>
                <div><dt>Still open</dt><dd>Could a different seat policy preserve that predictability while giving teams more room to collaborate?</dd></div>
              </dl>
            </article>
            <article className="public-pricing-card public-pricing-study" aria-labelledby="usage-study-heading">
              <span className="public-mini-label">Agent B / Experiment 02</span>
              <h3 id="usage-study-heading">Explore usage-based pricing</h3>
              <dl>
                <div><dt>Question</dt><dd>Can a bill that follows activity feel fair and remain easy to anticipate?</dd></div>
                <div><dt>Experiment</dt><dd>A second prototype compares quiet and busy months, showing how changes in activity affect spending without adding a charge for every collaborator.</dd></div>
                <div className="public-pricing-observation"><dt>Documented finding</dt><dd><strong>Flexible usage, uncertain bills.</strong> Teams can bring more people into the work. But a busy month makes the next bill harder to predict.</dd></div>
                <div><dt>Still open</dt><dd>Would an included allowance or a spending cap make usage-based pricing easier to budget for?</dd></div>
              </dl>
            </article>
          </div>

          <div className="public-pricing-synthesis" aria-labelledby="pricing-synthesis-heading">
            <div><p className="public-eyebrow">Connect the findings</p><h3 id="pricing-synthesis-heading">Neither approach resolves<br />both concerns.</h3></div>
            <div><p>When Agent C starts the pricing task, both investigations are available: predictable spending matters, and so does the freedom to invite collaborators.</p><p>The findings preserve what each approach gets right, where it falls short, and what is still unknown. Together, they give the next agent a reason to explore a third path.</p></div>
          </div>

          <article className="public-pricing-card public-pricing-proposal" aria-labelledby="pricing-proposal-heading">
            <div className="public-pricing-card-heading"><span className="public-mini-label">Agent C / Candidate decision</span><span className="public-pricing-badge">Awaiting human ratification</span></div>
            <h3 id="pricing-proposal-heading">Test a team plan with included usage</h3>
            <div className="public-pricing-proposal-grid">
              <div><h4>Why this is worth testing</h4><p>A team plan could let people collaborate without another seat charge. An included usage allowance could make spending easier to forecast. The proposal combines the strengths of both studies while keeping the remaining uncertainty visible.</p></div>
              <div><h4>What the next experiment checks</h4><ul><li>Do teams invite occasional collaborators more freely?</li><li>Can they anticipate spending in quiet and busy months?</li><li>Do they understand what happens when the included usage runs out?</li></ul></div>
            </div>
            <p className="public-pricing-review-note">A human reviews the reasoning before this experiment becomes the agreed next step. The final price remains an open question.</p>
          </article>
          <div className="public-pricing-follow"><p>Follow the findings back to their investigations, then inspect the proposed next test.</p><Link className="public-button public-button-light" href="/demo">Explore the pricing study <span aria-hidden="true">↗</span></Link></div>
        </div>
      </section>

      <section className="public-connect public-container"><p className="public-eyebrow">For the agents you already use</p><h2>Connect your agents today.</h2><p>Connect an agent through MCP and have it retrieve the project&apos;s decisions, investigations, and findings at the start of a related task.</p><div className="public-connect-labels"><span>Possibilities to explore</span><span>Evidence to compare</span><span>Decisions with reasons</span></div></section>

    </main>
    <footer className="public-footer public-container"><BrandLogo className="footer-brand-logo" /><p>AI should reduce burden, not authorship.</p>{workspaceLink && <Link href="/graph">Open the workspace ↗</Link>}</footer>
  </div>;
}
