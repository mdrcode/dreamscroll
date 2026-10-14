# Architecture & Implementation Plan: Screenshot-to-Knowledge-Graph Engine

## 1. Executive Summary & Core Philosophy

This document outlines the system architecture and design principles for an application that enables users to construct a personal knowledge graph from captured screenshots.

### Key Architectural Tenet: Zero Fine-Tuning
Training or fine-tuning custom Entity Linking (EL) models introduces heavy infrastructure overhead, continuous maintenance demands as real-world facts evolve, and unnecessary deployment costs. Instead, this architecture pairs **Vision LLM extraction at capture time** with **pre-computed Wikipedia/Wikidata embeddings**, **Wikidata SPARQL graph traversal**, and **targeted mini-LLM disambiguation/labeling**.

---

## 2. Architecture Overview & Resolution Pipeline

When a user captures a screenshot, the system executes a lightweight, three-tiered entity extraction and resolution process:

```
┌─────────────────┐
│   Screenshot    │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Vision LLM    │ ── Extract canonical name, category, surrounding context, 
└────────┬────────┘    and optimized search query
         │
         ▼
┌─────────────────┐
│ Wikipedia /     │ ── Free MediaWiki & Wikidata search APIs
│ Wikidata API    │    (wbsearchentities / opensearch)
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Mini-LLM        │ ── Conditional fallback: disambiguates candidate choices 
│ Disambiguation  │    only when ambiguity/low confidence occurs
└─────────────────┘
```

### Prompt Engineering Strategy for Vision Extraction
During initial screenshot processing, prompt the Vision LLM to return targeted queries rather than raw ambiguous text strings:

```json
{
  "entity_name": "Steve Jobs",
  "category": "Person",
  "context": "Co-founder of Apple Inc. and former CEO of Pixar.",
  "wiki_search_query": "Steve Jobs"
}
```

### Deterministic Identifier: Wikidata QIDs over Wikipedia URLs
While Wikipedia article URLs can change or redirect due to renames, Wikidata identifiers (**QIDs**, e.g., `Q19837` for Steve Jobs) remain permanent. Every node in the user's graph should be keyed by its QID.

---

## 3. Resolving Graph Sparsity: Pre-computed Entity Embeddings

A classic problem in recommendation engines is the **cold-start / sparsity problem**: when a user has captured only 1 to 3 nodes, traditional text embeddings or simple vector centroids point to isolated or unhelpful regions in vector space.

### Solution: Pre-computed Joint Knowledge Embeddings
Instead of embedding raw user text dynamically, the system imports pre-computed entity vector spaces (e.g., **Wikipedia2Vec** or Meta's **PyTorch-BigGraph Wikidata Embeddings**).

These pre-computed models combine three complementary signals into unified vector spaces:
1. **Textual Word Context:** Co-occurrence of terms across article bodies.
2. **Anchor Context:** Link neighborhood structures surrounding hyperlinks.
3. **Graph Topology:** Direct graph links across the Wikipedia/Wikidata page network.

### Resulting Advantage
On **Day 1, with Node #1 captured (`Q19837`: Steve Jobs)**, performing an $O(1)$ lookup for `Q19837` in the embedding index immediately yields dense, semantically accurate neighbors (*Steve Wozniak*, *Apple Inc.*, *NeXT*, *Pixar*) with zero cold-start delay and $0.00 in LLM token inference fees.

---

## 4. End-to-End User Journey: "Steve Jobs" $\rightarrow$ "Pioneers of Silicon Valley"

The following walkthrough illustrates how a user progresses from capturing a single concrete entity to receiving a macro-concept recommendation (*Pioneers of Silicon Valley*).

```
  [Capture 1: Steve Jobs (Q19837)]
                 │
                 ▼  (1-Hop SPARQL & Entity Vectors)
  [Suggested: Steve Wozniak & Apple Inc.]
                 │
                 ▼  (User Captures / Accepts)
  [Captures: Homebrew Computer Club & Fairchild Semiconductor]
                 │
                 ▼  (Multi-Vector Centroid + SPARQL Class Alignment)
  [Intersection Anchor: Silicon Valley (Q8639)]
                 │
                 ▼  (LLM Editorial Polish)
  [Macro-Concept Node: "📁 Pioneers of Silicon Valley"]
```

---

### Phase 1: Capturing Node #1 (`Steve Jobs`)

1. **User Action:** The user snaps a screenshot of an article mentioning Steve Jobs.
2. **Vision LLM Processing:** Image analysis extracts:
   ```json
   {
     "entity_name": "Steve Jobs",
     "context": "Co-founder of Apple Inc. and former CEO of Pixar.",
     "wiki_search_query": "Steve Jobs"
   }
   ```
3. **Entity Resolution:** The API queries `https://www.wikidata.org/w/api.php?action=wbsearchentities&search=Steve%20Jobs&language=en&format=json` and resolves to **`Q19837`**.
4. **Graph State:** The database stores **Node 1: `Q19837` (Steve Jobs)**.

---

### Phase 2: Instant Direct Neighbor Recommendations

Immediately upon adding Node 1, the system executes a low-latency graph expansion.

1. **Lookup & SPARQL Query:**
   * Fetch vector neighbors for `Q19837` from the pre-computed index.
   * Run a 1-hop SPARQL query against Wikidata to find direct relationships (`P112`: founder, `P178`: developer):
     ```sparql
     SELECT ?candidate ?candidateLabel WHERE {
       VALUES ?seed { wd:Q19837 }
       { ?seed ?p ?candidate . } UNION { ?candidate ?p ?seed . }
       SERVICE wikibase:label { bd:serviceParam wikibase:language "en". }
     } LIMIT 15
     ```
2. **Mini-LLM Display Synthesis:** A lightweight LLM formats the results for presentation:
   > *"Based on capturing Steve Jobs, you might want to add:"*
   > * **Steve Wozniak (`Q483381`)** — Co-founded Apple Inc. in 1976.
   > * **Apple Inc. (`Q312`)** — Technology company co-founded by Jobs.

---

### Phase 3: Graph Densification (4 Active Nodes)

Over several sessions, the user captures or accepts three more items:
* **Capture 2:** Homebrew Computer Club (`Q1089279`)
* **Capture 3:** Steve Wozniak (`Q483381`)
* **Capture 4:** Fairchild Semiconductor (`Q1138080`)

#### Active User Knowledge Graph (4 Nodes):
```
 [Q483381: Steve Wozniak] ─────── (Co-founded) ───────┐
            │                                         │
     (Attended)                                       ▼
            │                                 [Q19837: Steve Jobs]
            ▼                                         │
 [Q1089279: Homebrew Computer Club]                   │
            │                                         │
     (Geographic/Historical Context)                   ▼
            │                                 [Q1138080: Fairchild Semiconductor]
            └─────────────────────────────────────────┘
```

---

### Phase 4: Macro-Concept Synthesis Mechanics

Now that the graph contains multiple nodes, the system transitions from individual node expansion to **Macro-Concept Discovery**.

```
[ Captured Nodes ] ──► (Steve Jobs, Wozniak, Fairchild, Homebrew)
                             │
            ┌────────────────┴────────────────┐
            ▼                                 ▼
 1. VECTOR CENTROID                2. SPARQL ONTOLOGY TREE
    Calculates spatial average     Traverses shared properties
    in 100d/300d space             (P31 Class, P131 Location, P361 Part Of)
            │                                 │
            └────────────────┬────────────────┘
                             ▼
 3. MATHEMATICAL INTERSECTION & RANKING
    Filters SPARQL candidates by vector distance
                             │
                             ▼
 4. LLM EDITORIAL LABELING
    Transforms Q8639 ("Silicon Valley") into "Pioneers of Silicon Valley"
```

#### Step 4.1: Vector Centroid Calculation
Calculate the spatial average of the four active embeddings:

$$\vec{V}_{\text{centroid}} = \frac{1}{4} \left( \vec{V}_{\text{Jobs}} + \vec{V}_{\text{Wozniak}} + \vec{V}_{\text{Homebrew}} + \vec{V}_{\text{Fairchild}} \right)$$

Querying the vector index with $\vec{V}_{\text{centroid}}$ yields a cloud of ~50 candidate nodes centered around *1970s Northern California computing*.

#### Step 4.2: SPARQL Structural Alignment
Concurrently, query Wikidata for shared class ancestry and regional container nodes:

```sparql
SELECT ?commonClass ?commonClassLabel (COUNT(?item) AS ?matchCount) WHERE {
  VALUES ?item { wd:Q19837 wd:Q483381 wd:Q1089279 wd:Q1138080 }
  
  # Check shared locations, parent organizations, and classifications
  { ?item wdt:P131/wdt:P131* ?commonClass . }
  UNION
  { ?item wdt:P31/wdt:P279* ?commonClass . }
  
  FILTER(?commonClass NOT IN (wd:Q35120, wd:Q223557, wd:Q5)) # Exclude generic root classes like 'Human'
  SERVICE wikibase:label { bd:serviceParam wikibase:language "en". }
}
GROUP BY ?commonClass ?commonClassLabel
HAVING (?matchCount >= 2)
ORDER BY DESC(?matchCount)
```

#### Step 4.3: Hybrid Scoring & Selection
Intersect the vector candidates and SPARQL results using a combined scoring function:

$$\text{Score}(\text{Candidate}) = \alpha \cdot \text{CosineSimilarity}(\vec{V}_{\text{candidate}}, \vec{V}_{\text{centroid}}) + \beta \cdot \text{SPARQL\_Coverage\_Ratio}$$

Evaluating candidates against the active graph:

| Candidate Node | Cosine Sim | SPARQL Link Coverage | Hybrid Score | Status |
| :--- | :--- | :--- | :--- | :--- |
| `Q5` (Human) | 0.21 | 100% (Jobs, Wozniak) | 0.35 | Too Generic |
| `Q11658` (Semiconductor Industry) | 0.65 | 25% (Fairchild) | 0.52 | Too Narrow |
| **`Q8639` (Silicon Valley)** | **0.89** | **100% (All 4 Nodes)** | **0.91** | **Top Winner** |

---

### Phase 5: Decoupling Structural Anchor vs. Editorial Phrasing

It is essential to distinguish between the **mathematical anchor** and the **LLM editorial layer**:

#### 1. What the Latent Signals Provided (100% Mathematical / Structural)
* **Vector Math:** Proved that the four captured entities sit in the exact same dense neighborhood.
* **SPARQL Traversal:** Identified **`Q8639`** (*Silicon Valley*) as the shared structural container entity.

#### 2. What the LLM Editorial Layer Provided (Pragmatic Styling)
* **Raw Anchor:** `Q8639` (*Silicon Valley*) / Wikipedia Category: `Category:Silicon Valley people`
* **LLM Prompt:** *"Synthesize these 4 nodes (Jobs, Wozniak, Homebrew, Fairchild) anchored by entity Q8639 into a human-friendly collection title."*
* **LLM Output:** **`"Pioneers of Silicon Valley"`**

The LLM introduced the word *"Pioneers"* because it recognized that the cluster consists of early founders and foundational institutions. Grounding the LLM in `Q8639` guarantees factual relevance while producing polished UI copy.

---

## 5. Summary Architecture Matrix

| Strategy | Engine / Tool | Cost | Role |
| :--- | :--- | :--- | :--- |
| **Extraction** | Vision LLM | Low (~$0.001/img) | Extract canonical names & surrounding text context |
| **Resolution** | Wikidata Action API | $0.00 | Map extracted terms to permanent QIDs |
| **Embedding Search** | Pre-computed Vectors (Wikipedia2Vec) | $0.00 | Instant 1-hop & multi-vector candidate expansion |
| **Ontology Discovery** | Wikidata SPARQL Service | $0.00 | Identify structural parents, classes, and shared locations |
| **Macro Labeling** | Mini-LLM (e.g. Gemini Flash / Haiku) | Ultra-Low (<$0.0005) | Re-rank candidates and craft polished collection names |
```

***

### Summary of the Plan
This document captures:
1. **The Core Philosophy:** Avoiding custom model training by combining Vision extraction, free APIs, pre-computed vector space, and SPARQL.
2. **Cold-Start Resolution:** Utilizing joint Wikipedia embeddings so that even a single captured item yields rich semantic recommendations.
3. **The "Pioneers of Silicon Valley" User Flow:** A complete 5-stage progression from `Steve Jobs` to a structured macro-concept.
4. **The Latent vs. LLM Boundary:** Explaining how vector math and SPARQL establish the unshakeable anchor (`Q8639`: *Silicon Valley*), while the LLM supplies narrative UX copy (*"Pioneers of Silicon Valley"*).