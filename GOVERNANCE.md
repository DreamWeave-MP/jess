# The Constitution of Jess

*The fundamental principles of the Jess project, its engineering, its governance, and its community.*

## Preamble

We who build, maintain, and use Jess establish this Constitution to secure the freedom to create, to advance the practice of software engineering, and to ensure that the tools we construct remain servants of the people who use them.

We hold that software should be judged by what it demonstrably accomplishes, that technical authority carries a responsibility to withstand scrutiny, and that no implementation, convention, or institution is entitled to preservation merely because it already exists.

We recognize that progress requires experimentation; that experimentation requires the freedom to fail; and that lasting progress requires the discipline to distinguish successful experiments from convincing illusions.

We reject the proposition that complexity is evidence of sophistication, that tradition is evidence of correctness, or that the reputation of an author is evidence of the quality of their work.

We further recognize that engineering is a human endeavor. Its purpose is not the endless accumulation of machinery, but the expansion of what people can understand, create, experience, and share.

To these ends, we establish this Constitution.

---

## Article I — Purpose and Principles

### Section 1. The Mission

Jess exists to provide a capable, efficient, accessible, and extensible development environment based on Luau, supporting general-purpose software, games, simulations, and the tools necessary to create them.

Jess shall favor composable capabilities over unnecessary specialization, practical utility over architectural fashion, and measurable improvements over unsupported claims.

### Section 2. Freedom of Development

Jess shall not prescribe a single correct way to develop software.

Developers may work individually or collaboratively, employ automation or work manually, use supported third-party tools, and organize their applications according to their needs.

No artificial intelligence service, proprietary development platform, or private toolchain shall be required merely to use, build, or contribute to Jess.

### Section 3. Portability and Independence

Jess shall seek to minimize unnecessary dependencies, artificial platform restrictions, and exclusive reliance on individual vendors.

Portability shall be pursued where technically, legally, and practically feasible. Support for a platform shall be established by implementation and verification, not assertion.

### Section 4. The Primacy of Purpose

Performance, abstraction, extensibility, and technical elegance are instruments, not ends in themselves.

When these interests conflict, decisions shall be made in consideration of the actual needs of users, the correctness of the software, and the long-term sustainability of the project.

---

## Article II — Evidence and Engineering

### Section 1. No Sacred Implementations

No implementation, architecture, language, dependency, or established practice shall be considered immune to replacement or revision.

Prior investment, popularity, authorship, and historical significance may inform the cost of a change, but shall not independently establish its technical merit.

### Section 2. The Burden of Proof

Those making material technical claims shall be prepared to substantiate them through evidence appropriate to the claim.

Performance claims shall be supported by representative measurements. Correctness claims shall be supported by appropriate testing or analysis. Compatibility claims shall be supported by verification against the relevant contracts and environments.

Usability, maintainability, accessibility, and other qualities not readily reducible to numerical measurements shall be evaluated through reasoned analysis, practical examples, and relevant experience.

Absence of evidence shall not be represented as evidence of success.

### Section 3. Reproducibility and Intellectual Honesty

Published experimental results shall identify sufficient methodology, conditions, and limitations to permit meaningful independent scrutiny.

Where independent reproduction is impractical, those limitations shall be disclosed.

Measurements shall not be selectively represented to conceal material regressions, failures, uncertainties, or tradeoffs.

Results that contradict expectations shall be reported faithfully.

### Section 4. Correctness Before Performance

An optimization that violates the required behavior of a system is not an optimization.

Performance improvements shall be evaluated against correctness, safety, resource consumption, portability, complexity, and maintenance costs.

The fastest implementation is not necessarily the best implementation.

### Section 5. Revision by Evidence

Decisions shall remain open to reconsideration when new evidence, requirements, or circumstances materially alter their justification.

An established decision is a record of prior judgment, not a prohibition against future understanding.

---

## Article III — Contributions and Experimentation

### Section 1. The Right to Propose

Any person may publicly propose an improvement, identify a defect, challenge a technical claim, or suggest an alternative through the project's official repositories and contribution channels.

**All official contribution proceedings shall be conducted in the open.**

The project shall maintain no private contribution channels, privileged deliberative forums, or undisclosed procedures through which contributors may obtain official acceptance of changes, preferential consideration, or governing authority.

Contributors remain free to develop ideas, conduct experiments, and prepare implementations privately or within their own repositories. Such work acquires no official standing until submitted through the project's public processes.

Proposals shall be assessed according to their substance, relevance, and supporting evidence, not the reputation, seniority, identity, or preferred tools of their authors.

This right to propose does not establish an unconditional right to acceptance, review on demand, or consumption of project resources.

### Section 2. Freedom to Experiment

Experimental work shall not be burdened with the same stability and compatibility obligations as publicly supported interfaces.

Prototypes, exploratory packages, and unconventional approaches are legitimate forms of engineering.

The project shall encourage experiments that can be evaluated, challenged, improved, or abandoned without unnecessary institutional friction.

### Section 3. Responsibility Upon Adoption

When an experiment becomes a supported project component, its maintainers shall assume appropriate obligations for correctness, documentation, security, compatibility, and maintenance.

The maturity and support expectations of publicly distributed components shall be communicated clearly.

Promotion to stable status shall follow demonstrated reliability rather than mere age, popularity, or enthusiasm.

### Section 4. Constructive Review

Technical criticism shall identify actionable concerns wherever reasonably possible.

A blocking review shall communicate the grounds for rejection or required revision with sufficient clarity that its reasoning can be understood and challenged.

Disagreement is not misconduct. Technical criticism is not personal hostility.

Contributors shall remain responsible for understanding and substantiating the changes they submit.

### Section 5. Proportional Process

Routine changes shall not require architectural ceremonies, elaborate proposals, or formal decision records when ordinary review and testing are sufficient.

Changes with significant consequences for public interfaces, security, portability, or architectural direction may require additional documentation and review proportionate to their impact.

---

## Article IV — Governance and Stewardship

### Section 1. Founding Stewardship

Jess is a founder-led project.

The founder retains ultimate responsibility for the project's direction, official releases, architectural priorities, and delegation of maintenance authority.

This Constitution does not establish governance by popularity, voting, or committee.

Authority shall be exercised in service of the project's stated purpose, rather than treated as evidence of technical infallibility.

### Section 2. Delegated Authority

The founder may delegate responsibility for repositories, packages, subsystems, releases, or other defined areas to maintainers.

A **maintainer** is a member of the `jess` team of the DreamWeave-MP GitHub organization who also holds the maintainer role in that organization. The team's membership is the public record of delegation. A delegation narrower than the whole project shall be recorded with the team membership or in the repository it concerns.

Delegated authority should be accompanied by a clear understanding of its scope and responsibilities.

Maintainers are accountable for decisions made within their areas of responsibility and may be subject to review or replacement when necessary.

An **official release** is one built, signed, and published by the official repository's release pipeline from a tag pushed by a person holding release authority. Release authority belongs to the founder and the designated successor. The founder may extend it to a maintainer by recording the delegation; no one else may cut a release in the project's name.

### Section 3. Open Deliberation and Accountable Decisions

**The business of governing Jess shall be conducted in public.**

The official public venue for the business of the project is the Jess repository on GitHub, `DreamWeave-MP/jess`: its issues, pull requests, and discussions. No other space, public or private, carries official standing. Procedural matters, the mechanics of continuous integration, releases, and publishing, may be delegated to StroggForge, the organization's tooling repository, whose procedures are public.

Proposals concerning project policy, architectural direction, contribution standards, and other material matters of governance shall be deliberated in the official venue.

The founder and any delegated maintainers shall exercise their official authority openly, recording material decisions and their reasoning in the official venue, as an issue, a pull request, or a discussion, so that the record and the commit history together account for every material decision.

Private conversations, personal relationships, or undisclosed agreements shall confer no special governing authority and shall not substitute for public deliberation.

The project may withhold sensitive security disclosures, private personal information, legally protected communications, or other information where publication would cause material harm or violate legitimate obligations. Such exceptions shall be construed narrowly and shall not serve as a pretext for secret governance.

Where confidential circumstances require official action, the decision and its justification shall be disclosed publicly to the extent reasonably possible once doing so is safe and lawful.

The founder retains final decision-making authority as established by this Constitution. **The existence of that authority does not diminish the obligation to exercise it transparently.**

### Section 4. The Right to Challenge Authority

Technical claims made by the founder or maintainers may be challenged on the same evidentiary grounds as claims made by any other contributor.

Such challenges shall not, by themselves, be grounds for retaliation or exclusion.

The founder retains final decision-making authority, but shall not represent personal preference as experimental fact or silence contrary evidence merely to preserve authority.

Disputes climb one step at a time. A disagreement between contributors is decided by the maintainer responsible for the area; between maintainers, or between a maintainer and a contributor over that maintainer's decision, by the founder or, in the founder's absence, the acting steward. Each decision is recorded in the official venue with its reasons.

### Section 5. Continuity of Stewardship

The founder has designated a successor: **Vidi Aquam** (`VidiAquam` on GitHub). The founder may change the designation, or name further successors in order, by amendment.

The founder is **absent** when ninety days pass with no official action by the founder: no commit to the official repository, no official release, and no decision recorded in the official venue. On the ninety-first day, succession proceedings begin: the designated successor assumes stewardship, with every power this Constitution gives the founder, by recording the assumption and its date in the official venue. No further notice or waiting period is required; three months is notice enough.

If no designated successor is able to serve, the maintainers choose an acting steward from among themselves and record the choice in the official venue. If there are no maintainers, the project is unmaintained: whoever holds the organization shall say so in the official repository, and the provisions on the name (Article V, Section 5) continue to apply.

A founder who returns keeps their commit rights. Whether the founder resumes stewardship is for the founder and the acting steward to decide together, and the decision is recorded in the official venue. Until they decide, the acting steward continues.

Changes to the structure of constitutional authority shall be made through the amendment procedure established herein.

---

## Article V — Compatibility and Public Trust

### Section 1. Explicit Commitments

Jess shall distinguish experimental capabilities from interfaces on which users are reasonably expected to rely.

Compatibility and support commitments shall be documented, appropriately scoped, and honored according to the maturity of the component.

No undocumented promise of universal portability, perpetual support, or unrestricted compatibility shall be inferred merely from the project's ambitions.

### Section 2. Responsible Change

Breaking changes to supported interfaces shall carry a technical justification and be communicated through an appropriate release process.

Where practical, migration paths and reasonable deprecation periods should be provided.

Urgent corrections involving serious security, legal, or correctness defects may require accelerated changes.

### Section 3. Honest Releases

Official releases shall accurately communicate known limitations and material changes.

A release shall not be declared stable solely to satisfy a schedule, marketing objective, or numerical versioning milestone.

Release claims shall be supported by verification appropriate to their scope.

### Section 4. Sustainability

No contributor or maintainer shall be expected to provide unlimited labor, indefinite availability, or unconditional support.

The project shall prefer explicit maintenance commitments to implied obligations that cannot reasonably be fulfilled.

### Section 5. The Name

Jess is the name of this project and of the software its official releases contain. The licenses let you fork it, change it, sell it, embed it, and keep your changes to yourself. Do that freely.

Do not call the result Jess. A derivative that is not an official release shall not be named Jess, present itself as Jess, or use the name in a way that implies it is the official project or endorsed by it. Saying that a thing is built on Jess, or forked from Jess, is fine: it is true.

The name goes with the stewardship. Whoever holds stewardship under Article IV may use it in the project's name; no one else may. This is the one thing the project asks of everyone who takes its code.

---

## Article VI — Security, Provenance, and Development Methods

### Section 1. Security Responsibility

Security boundaries, capability restrictions, dependency risks, and other material threats shall receive scrutiny appropriate to their potential consequences.

Credible vulnerability reports shall be treated seriously, with sensitive information handled responsibly.

The project may establish confidential procedures for vulnerability reporting and remediation without making unrelated internal development methods public.

### Section 2. Licensing and Attribution

Jess shall respect applicable software licenses, contributor rights, attribution requirements, and other legal obligations.

Neither automation nor technical transformation shall excuse the inclusion of code or assets without appropriate rights.

Source provenance material to licensing, security, or compliance shall not be knowingly misrepresented or concealed.

Unless stated otherwise at the time, any contribution intentionally submitted for inclusion in the project is licensed as the project is, MIT OR Apache-2.0, without additional terms. Submitting it is the statement.

### Section 3. Freedom of Method

Jess recognizes both human-directed manual development and development assisted by automated systems, including artificial intelligence.

No implementation shall be accepted solely because of its method of production, nor rejected solely because of that method.

The person or maintainer responsible for a contribution remains accountable for its correctness, provenance, suitability, and compliance with project standards.

### Section 4. The Boundary of Disclosure

The project's public obligations concern its published software, technical claims, documented interfaces, licensing, and governance.

Private development workflows, agent instructions, coordination methods, internal skills, and personal tooling need not be published merely because they were employed in producing the software.

This protection does not override applicable legal obligations or justify withholding information materially necessary to evaluate a published claim.

### Section 5. Independence from Automation

Jess shall not define itself by the use of any particular development methodology or automation technology.

Its value shall be established through the capabilities and quality of the software it provides.

**Jess is built for developers. The choice of development tools belongs to them.**

---

## Article VII — Community and Professional Conduct

### Section 1. Respect for Persons

Jess shall encourage vigorous criticism of ideas, implementations, measurements, and technical decisions while maintaining respect for the people involved.

Harassment, threats, discriminatory abuse, deliberate sabotage, and retaliation are incompatible with participation in the project's official spaces.

Standards of conduct shall be applied consistently, including to maintainers and the founder.

### Section 2. The Freedom to Be Wrong

Mistakes made in good faith shall not be treated as moral failures.

Contributors are expected to correct errors, acknowledge valid evidence, and engage constructively with review.

The discovery of an error is an opportunity to improve the project, not an occasion to assign humiliation.

### Section 3. Expertise and Accessibility

Expertise shall be respected but shall not confer immunity from scrutiny.

Beginners shall not be expected to possess the experience of established maintainers.

Reasonable efforts shall be made to explain technical expectations clearly, without requiring maintainers to provide unlimited individual instruction.

### Section 4. Enforcement

Project stewards may moderate official project spaces and restrict participation when necessary to protect contributors, users, or the project.

Material enforcement actions should be proportionate, based on identifiable conduct, and accompanied by reasons where doing so is safe and appropriate.

Where practical, affected participants should have an opportunity to seek reconsideration. An independent reviewer should be used where one is available and appropriate.

---

## Article VIII — The Limits of Bureaucracy

### Section 1. The Burden of Justification

Every mandatory process, policy, approval requirement, and administrative obligation shall serve an identifiable purpose.

A proposed requirement should answer three questions:

1. What problem does this requirement prevent or resolve?
2. What evidence would indicate that it is succeeding or failing?
3. Is there a less burdensome means of achieving the same protection?

When these questions cannot be answered reasonably, adoption should be deferred.

### Section 2. Minimum Necessary Process

The project shall prefer the simplest procedures capable of protecting its legitimate interests.

Administration shall not become an end in itself.

Additional organizational hierarchy, mandatory documentation, or review stages shall not be introduced solely to imitate the practices of larger institutions.

### Section 3. Review and Removal

Established procedures shall remain open to simplification, consolidation, suspension, or repeal.

A rule that repeatedly obstructs productive work without providing proportionate benefits should be reconsidered.

No procedure shall justify its continued existence solely by citing its own prior adoption.

### Section 4. No Doctrine Beyond Evidence

This Constitution establishes enduring principles of judgment and conduct.

It does not prescribe permanent technical architectures, preferred algorithms, organizational fashions, or implementation languages beyond the defined purpose of Jess.

**The project's methods must remain free to improve.**

---

## Article IX — Amendment and Ratification

### Section 1. Authority to Amend

Amendments may be proposed by any contributor through the project's public contribution channels.

The founding steward retains authority to ratify amendments until that authority is delegated or reconstituted by amendment.

### Section 2. Public Reasoning

Material constitutional amendments shall be proposed in the official venue, accompanied by a statement of their purpose and anticipated consequences, and left open for comment for at least fourteen days before ratification. An amendment is material when it changes who holds authority, how it is exercised, what the project commits to, or what this Constitution covers; correcting wording that changes none of those is not.

Urgent amendments needed to address immediate security, legal, or operational risks may be adopted provisionally, subject to subsequent public explanation and reconsideration.

### Section 3. Preservation of History

Ratified amendments shall be recorded in the public history of this document.

Superseded provisions shall remain discoverable through version control.

Changes shall not be concealed by rewriting the historical record of governance.

### Section 4. Constitutional Supremacy

This Constitution governs the conduct and decision-making of the officially maintained Jess project: the `DreamWeave-MP/jess` repository and what its releases contain, and every Jess package repository owned and managed by the DreamWeave-MP organization. A project outside the organization may adopt it by reference; nothing here binds one that doesn't.

Project procedures and maintainer decisions should be interpreted consistently with its principles.

Nothing herein supersedes applicable law, published software licenses, or the rights conferred by those licenses.

### Section 5. Ratification of Amendments

An amendment is ratified by a commit to this file on the default branch of the official repository, made by the founder or the acting steward after the comment period of Section 2, that carries the amended text and raises the version below. Nothing else is ratification. An amendment takes effect on ratification unless it says otherwise.

---

## The Founding Commitment

We establish Jess not as a monument to an implementation, a language, a founder, or a particular method of engineering.

We establish it as a means by which people may create things that would otherwise remain beyond their reach.

We recognize that our understanding is incomplete, that our measurements may be mistaken, that our decisions may be overturned, and that future contributors may achieve what we could not.

We welcome these possibilities.

For the purpose of this project is not to preserve our victories, but to make further progress possible.

**No claim above challenge.**

**No person beneath respect.**

**No tool above its purpose.**

And no rule beyond reconsideration.

*Established in the service of those who build, and those who will build after us.*

---

**Status:** Ratified 10/21/2026

**Custodian:** S3kshun8

**Constitutional version:** 1.0
