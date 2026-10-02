# フェーズ 4 実装計画：interpret と方策の整合（作業文書）

この文書は、フェーズ 4 の設計比較（試作 A: コンパイル時排他、B: 方策鏡像、C: 実行時排他 + 調整）と 2 名の審査を統合した実装計画である。各レーンは本文を 05/06/07/09/11/12/13 の各設計文書に取り込む。取り込みが終わった後も、判断の経緯の記録として残す。決定事項・レーン・受け入れ基準・基準改訂の節は、統合時の英語原文のままとする。

## 試作の実測（要約）

### A-compile（wip/p4d-A-compile 57bd61d）

- ESS suite median ConstraintProposal ESS/n overall (n=1000, 50 auctions, SAMPLE_SEED 0xE55): 0.0358 (loadavg 6.70) → 0.0863 on final HEAD (loadavg 10.95). Other runs: 0.0949 after the rank policy step, 0.0948 after the flat complements step. Across the three runs the median moves by about 0.01 from run to run. Overall mean 0.102 -> 0.156, geometric mean 0.041 -> 0.068.
- ESS suite median ESS/n generated / corpus: generated 0.0403 / corpus 0.0302 → generated 0.1269 / corpus 0.0843 (final). Intermediate steps: exclusive system calls only 0.0584/0.0375; plus rank policy and natural exclusion 0.1096/0.0650; plus flat complements 0.1096/0.0706.
- ESS policy breakdown, generated (exact on/shared/off_inside/off_outside; natural the same): exact 53953/18744/3125/21178; natural 44358/8578/24770/13294 → exact 75160/0/0/21840; natural 58028/401/0/32571
- ESS policy breakdown, corpus: exact 48463/16967/3663/15907; natural 36620/8640/51075/23665 → exact 60692/0/0/20308; natural 48408/698/11750/51144. The remaining natural off_inside are natural calls at on-system positions, which the policy never makes for any hand.
- ESS diagnostic experiments (step-2 code plus reverted env toggles; ConstraintProposal/mixture changes outside this angle): 0.0949 (gen 0.1096, corpus 0.0650) → components drawn by w_i only: 0.1741 (0.2297/0.0580); plus no middle-seat coarsening: 0.2404 (0.3084/0.0998), suite 31 s; plus eps_natural=0.05: 0.3239 (0.3778/0.0454)
- ESS suite wall time (release, test body only): 6.0 s → 7.1 s (loadavg about 11)
- cargo test -p bridge-bidding --all-features (includes consistency 1e3 and policy): all pass (consistency 5 passed / 2 ignored, policy 4 passed / 1 ignored) → all pass, plus the new tightness test (1 passed / 1 ignored). Whole workspace (--exclude bridge-dds --all-features) passes; clippy -D warnings is clean; wasm32 check passes.
- Release forward consistency, 10^5 positions (SAYC_CONSISTENCY_N=100000, seed 0x5a1c0002): 111 violations, all gap_induced, 0 not gap-induced; chosen=85561, no_candidate=234, implicit_pass=14205; 6.09 s (loadavg 10.49) → 111 violations, all gap_induced (the same forced-Pass gaps), 0 not gap-induced; identical position counts; 7.05 s (loadavg 7.05)
- Policy argmax == choose_bid, 10^5 SAYC positions (release, ignored test): passes (tau 0.01) → passes (tau 0.01). The default rank policy (tau 0.2) also passes the 1e3 test.
- Tightness, 2000 positions x 100 random hands [inside&picked, inside&not picked, outside&picked] for exact / partial / natural: exact [25069, 5873, 0]; partial [6, 0, 0]; natural [42281, 25733, 0] → exact [25069, 0, 0]; partial [6, 0, 0]; natural [42281, 0, 0]
- interpret bench, synthetic 12-call / realistic 12-call: 10.38 us / 11.82 us (loadavg 5.94 -> 7.48) → 10.97 us / 12.34 us (loadavg 8.69 -> 5.76). A later run gave 14.1 / 12.6 at loadavg 5.6 -> 9.4.
- interpret bench, SAYC 1NT / competitive / 12-call: 9.92 us / 7.33 us / 26.9 us → with exclusive_natural (default): 73.9 / 44.5 / 97.0 us; with exclusive_natural=false (system exclusion only): 9.92 / 8.79 / 16.4 us
- sequence_log_likelihood bench, 12-call / realistic: 6.23 us / 6.45 us → 5.41 us / 6.02 us
- Natural-position cost breakdown (SAYC, 1NT P 2C, 31 legal calls): choose_bid natural branch 13 us → classify for 31 calls 1.9 us + infer for 31 calls 10.2 us, paid once per natural call in interpret. The exact DNF form of the natural exclusion cost about 20 us more and was replaced by the tree form.
- SAYC compile time (release): 85 ms → 90 ms (index build 4.6-10 ms); largest vendor system (wj/1C.bml) 11.5 ms -> 11.7 ms
- Reproduction harness (SAYC_REPRO_LIMIT=100): median 0.0 (floored and overall), mean rate 0.0239, 3 auctions > 0.01; Exact-kind median 0.881; ESS median 2.79 (loadavg 6.77) → median 0.0, mean rate 0.0299, 3 auctions > 0.01; Exact-kind median 0.996; ESS median 1.98 (loadavg 6.64). 97/100 corpus auctions contain a natural call at an on-system position.

### B-mixture（wip/p4d-B-mixture (worktree /private/tmp/claude-501/-Users-hirotosasaoka-Documents-bridge-engine/a42df94e-72dc-44a4-af9d-faf50f013d4f/scratchpad/wt/p4d-B-mixture, based on wip/p4proto-base 7c390cd; 12 commits, 11 files, +1615/-71; never pushed or merged) b2743b9）

- ESS suite median ESS/n overall / generated / corpus (ConstraintProposal, tau=1, 50 cases): 0.0358 / 0.0403 / 0.0302; 2 of 50 cases >= 0.5 (legacy interpret) → Mirror + literal-free coarsen: 0.4264 / 0.5029 / 0.3769, 23 of 50 >= 0.5, acceptance 1.0. Mirror + coarsen + residual rejection: 0.9055 / 0.9299 / 0.8819, 40 of 50 >= 0.5 (criterion met). loadavg about 4-16.
- ESS per attempt (accounts for residual-rejection cost): legacy: equal to ESS/n (no rejection); legacy + residual rejection: 0.0283 → mirror + residual: 0.3706 overall (generated 0.5131, corpus 0.1309); acceptance median 0.519, min 0.002 (worst case produced 132 of 1000 deals); suite wall time 8 s -> 12-15 s
- ESS ablations (median ESS/n overall): legacy 0.0358; legacy + residual 0.0511; mirror with old coarsen 0.3577 → tau=0.25: legacy 0.0296, mirror 0.4527, mirror + residual 0.9155 (ESS per attempt 0.4132). merge_ratio 1.05: 0.9054; 2.0: 0.8935. K=4: 0.7983; K=2: 0.0434 (collapse).
- Policy breakdown, exact calls, generated set (on / shared / off_inside / off_outside): 53953 / 18744 / 3125 / 21178 → 80907 / 14924 / 2 / 1167 (mirror + residual)
- Policy breakdown, natural calls, generated set: 44358 / 8578 / 24770 / 13294 → 75622 / 15155 / 209 / 14
- Policy breakdown, natural calls, corpus set: 36620 / 8640 / 51075 / 23665 → 64410 / 7878 / 6522 / 33098 (off_outside still high: Natural constraints with literals/Custom are over-covered only)
- cargo test --workspace --exclude bridge-dds --all-features and cargo test -p bridge-bidding --all-features: pass (base 7c390cd) → pass, including new mirror_matches_call_distribution and sayc_forward_consistency_mirror_1e3. dds_smoke dds_backend_gives_one_group_for_a_single_suit_hand is flaky and fails on base too with DDS vendored. Clippy (-D warnings), fmt and wasm32 check are clean.
- Release forward consistency 1e5 (strict): violations by cause, time: legacy: gap-induced 111, non-gap 0; 4.6 s → mirror: gap-induced 119, non-gap 0; 8.8 s
- Release forward consistency 1e6 (strict): legacy: gap-induced 1279, non-gap 0; 45 s → mirror: gap-induced 1357, non-gap 0; 87 s
- Mirror accuracy vs call_distribution (1000 deals, 17860 calls, 375060 call-hand checks): n/a (legacy interpret has no mass guarantee) → tau=1: exact 372632 (99.35%), over-covered 2428, under-covered 0; alternatives per call mean 5.01, max 33; seat levels mean 6.39, max 8. tau=0.25: alternatives mean 3.46, max 21; seat levels mean 5.91.
- interpret bench, hand-built 12-call / 12-call realistic: 10.46 us / 11.76 us (legacy) → 7.84 us / 8.40 us (mirror, memoised)
- interpret bench, SAYC cases: 1nt / competitive / 12-call: 9.35 / 7.39 / 17.1 us (legacy) → 96.9 / 65.4 / 144.6 us (mirror, memoised). sayc-12 uncached about 205 us: enumerate 71, split 85, product 28. Off-system about 15 us per call, not cacheable. 10 us target missed.
- Reproduction harness, 100 corpus auctions: uniform: median rate 0.0, 7 auctions with ESS >= 30, median ESS 2.8, 36 s → mirror + ConstraintProposal + residual: median rate 0.0, 92 with ESS >= 30, median ESS 831, 174 s. tau=0.1: 93 with ESS >= 30, mean rate 0.098 (14 non-zero, 11 >= 0.6); 97 of 100 auctions contain Natural calls. The rate is a policy/system property, not fixed by interpretation.

### C-runtime（wip/p4d-C-runtime ae6a4f5）

- ESS suite median ESS/n overall (50 auctions, 1000 samples each, release): 0.0358 (base 7c390cd; uniform 0.0030) → 0.3013 (05808c6, the last code-changing ESS run; ae6a4f5 only adds tests/comments; uniform 0.0038; loadavg 13.81 9.54 9.88)
- ESS median ESS/n generated: 0.0403 → 0.2982 (note: the generated case set changed because the level floor alters the generator's auctions)
- ESS median ESS/n corpus: 0.0302 → 0.3498
- ESS per-auction distribution (auctions >=0.5 / >=0.25, per 25): generated 1/3, corpus 1/4 → generated 6/14, corpus 8/13
- policy breakdown generated exact {on, shared, off_inside_node, off_outside_node}: {53953, 18744, 3125, 21178} → {81138, 0, 122, 17740}
- policy breakdown generated natural {on, shared, off_inside_node, off_outside_node}: {44358, 8578, 24770, 13294} → {85249, 0, 986, 7765}
- policy breakdown corpus exact {on, shared, off_inside_node, off_outside_node}: {48463, 16967, 3663, 15907} → {66882, 0, 90, 18028}
- policy breakdown corpus natural {on, shared, off_inside_node, off_outside_node}: {36620, 8640, 51075, 23665} → {72180, 0, 15988, 31832}
- corpus calls read as Fallback-only (500 corpus auctions, 5696 calls): 0/5696 → 72/5696 (shadowed off-policy calls; keeping them as un-excluded regions was measured at corpus ESS 0.1490, so it was rejected)
- cargo test -p bridge-bidding --all-features: all pass (base) → 93 passed, 0 failed (ae6a4f5). Workspace clippy (--exclude bridge-dds --all-targets --all-features -D warnings) is clean; the wasm32-unknown-unknown check passes; bridge-lead dds_smoke fails both before and after (pre-existing)
- consistency 1e5 positions release (seed 0x5a1c0002): 111 violations, all gap-induced, 0 non-gap; chosen=85561 no_candidate=234 implicit_pass=14205; 4.78 s (loadavg 6.84 8.52 9.43) → 0 violations (0 gap-induced, 0 non-gap); chosen=82649 no_candidate=28 implicit_pass=17323; 9.68 s (loadavg 10.65 9.24 9.76)
- consistency 1e3 (seed 0x5a1c0001): 0 non-gap violations → 0 violations; chosen=818 no_candidate=0 implicit_pass=182
- criterion interpret/12-call-auction (median): 10.543 µs (loadavg 6.81 8.17 9.16) → 11.440 µs (loadavg 8.26 8.61 9.37)
- criterion interpret/12-call-auction-realistic: 12.191 µs → 12.890 µs
- criterion interpret/sayc-1nt-auction: 9.311 µs → 12.676 µs (+36%)
- criterion interpret/sayc-competitive-auction: 7.363 µs → 7.976 µs
- criterion interpret/sayc-12-call-auction: 17.532 µs → 17.750 µs (an intermediate prototype was 57.2 µs before impure-Not dropping, the atom cap and batching)
- criterion sequence_log_likelihood/12-call-auction: 6.174 µs → 5.200 µs
- harness interpret warm / cold (sayc-1nt, sayc-competitive, sayc-12-call, gen-22, gen-13): warm 9.36/7.35/17.30/11.40/17.81 µs; cold 9.4/7.3/18.5/11.5/17.9 µs (no cache) → warm 11.24/7.35/17.67/13.04/17.79 µs; cold 140.2/136.3/240.2/134.4/275.8 µs (first touch fills the exclusion cache; loadavg 8.35 7.91 9.36)
- harness seq_ll per deal (same 5 auctions): 48.13/41.66/92.29/38.82/93.12 µs → 51.82/46.24/99.42/43.20/131.71 µs
- replay escalation: final-contract level histogram over 2000 generated deals [passout,1..7]: [39,47,189,219,150,37,20,1299] (measured with the new tau 0.1 policy but no level floor; cue/raise/rebid_own ladders) → [39,47,468,944,368,116,16,2]
- reproduction harness (SAYC_REPRO_LIMIT=200): median rate 0.0000 (15 auctions with ESS>=30); ESS median 2.65; 64.5 s → median rate 0.0000 (13 auctions with ESS>=30); ESS median 3.01; 51.5 s. Unchanged because the harness uses UniformProposal on human auctions

## 審査

### 審査 0（勝者: B-mixture）

- B-mixture: 8
- C-runtime: 6
- A-compile: 5

移植する要素:

- From A: compile-time precompute of sibling groups keyed by (parent TrieId, seat/vul condition class) with Lookup.parent. Use it to precompute or cache mirror partitions and the implicit-pass complement per group instead of B's raw-pointer-keyed MirrorCache. This removes the stale-pointer risk and most of the 150 us SAYC cost.
- From A: a single explicit rank comparator (priority, then tie-break, then call index) shared by choose_bid, call_distribution and the interpretation, plus the shadowed-branch lint (246 SAYC branches never reachable).
- From A: the bidirectional tightness test, generalised for the mirror. For random hands, assert under-cover == 0 and exact >= 95% against call_distribution, and add the argmax-region check at deterministic tau. Make it a default-suite guard.
- From C: validate the cache with Arc::ptr_eq on Table.systems and the natural engine, and add a per-prefix natural-position cache so off-system mirror calls are not recomputed.
- From C: synthesise an implicit Pass in choose_bid's natural branch under ImplicitPass::Complement. It removes the forced-Pass gaps (no_candidate 234 -> 28, 0 gap-induced violations at 1e5) and makes the mirror's no-candidate 1/n region rarer.
- From C: the natural level floor for replay escalation (root cause 3), as a separate, individually validated change to the natural policy and the generator, not bundled with interpretation work. It changes the target, so measure it on a held-out auction set.
- Process: report ESS per attempt next to ESS/n and put an attempt budget on residual rejection. Judge the phase-5 criterion on mirror-only ESS/n (0.43) or on per-attempt ESS, not on the 0.906 post-rejection figure.
- Enforce MirrorParams == BidContext.policy by construction, for example by deriving the mirror params from the BidContext passed to the sampler, so interpretation and likelihood cannot drift apart.

### 審査 1（勝者: C-runtime）

- C-runtime: 7
- B-mixture: 6
- A-compile: 4.5

移植する要素:

- From B: the interpretation-agnostic ConstraintProposal changes: literal-free coarsen (is_literal_free_atoms) and residual_rejection. I applied B's patch to C and measured ESS/n 0.3013 -> 0.6077 (gen 0.7068, corpus 0.4482) with residual rejection, and 0.3265 without it; ESS per attempt was 0.287. It needs an attempt budget and ESS-per-attempt reporting in the suite before becoming the default.
- From B: the mirror accuracy test pattern (under-cover == 0 against call_distribution over random hands) as a probabilistic gate, plus the enumerate_candidates refactor so choose_bid, call_distribution and interpret share one hand-independent candidate list. That removes C's (and A's) duplicated natural ranking.
- From B: the insight that interpretation weights should carry policy mass. C's eps 0.003 approximates it; weighting each branch by its policy probability (at minimum shrinking the ANY Fallback weight to about eps/n) is the principled version.
- From A: tests/tightness.rs as a CI gate. It ports unchanged onto C and currently shows 0.6%/0.7% slack, which gives a measurable target for the impure-sibling and 4-atom-cap widening.
- From A: Lookup.parent (tracked for free in resolve) to replace C's extra resolve(parent_key) on every cache miss. Also the explicit call-index final tie-break shared by sort_kept and the exclusion ranking.
- From A: precompute the system-node exclusion per (parent TrieId, seat/vul class) sibling group, to remove C's 134-276 us cold cost and the unbounded node cache. Build it lazily in a OnceLock keyed to an immutable Arc<SystemIR> rather than serialising it into the IR, which avoids A's measured 1.51x postcard growth and the IR_FORMAT bump. Keep only the natural-prefix cache bounded (LRU).
- From A: the shadowed-branch lint (246 SAYC branches choose_bid can never reach), which also justifies C's Fallback-only reading of shadowed calls.
- From A: consider its rank-based policy score (-rank/tau) in place of C's ad hoc tie_gap. It gives argmax == choose_bid for any tau and has one parameter instead of two. Re-tune it on a held-out auction set, not on the ESS suite.
- Unify C's exclusion.rs Grid ([ShapeSet;38]) and B's grid.rs (HCP runs) into one exact shape x HCP set module.

## 設計本文（07-bidding.md ほかへの差し替え・追記）

以下は docs/design/07-bidding.md への差し替え・追記の本文（貼り付け用）と、06/05/09/11/12/13 への注記である。見出し番号は 07-bidding.md の既存の節に合わせてある。「差し替え」と書いた節は、既存の本文をこの内容に置き換える。

==================================================================
07-bidding.md
==================================================================

【冒頭の決定要約に追記】
フェーズ 4 で、方策 `call_distribution` は「システムの決定的選択 + ナチュラルへの逸脱 δ + 一様床 ε」に改める（温度付きソフトマックスは廃止）。`interpret` はこの方策を写したもの（方策鏡像）に改める。コール c の解釈の密度 Σ_i w_i·1[h ∈ C_i] は、その位置での p(c | h) にコールごとの定数倍で一致する。システムコールの排他領域は `bridge-system` の `ExclusiveIndex` に派生データとして前計算する（直列化はしない。`IR_FORMAT` は変えない）。

---------------------------------------------
§2（`bridge-system` から消費するインターフェース）に追加
---------------------------------------------
- `Lookup.parent: TrieId`：`matched_depth − 1` のトライノード。`resolve` のループの中で追跡するので追加コストはない。`interpret` は、いま一致したコールの兄弟集合を引くのに使う（厳密一致と `resolve_lenient` の各試行の両方）。
- `bridge_system::exclusive`
  - `rank_cmp(sys, a, b)`：システム候補の全順序。**priority 降順 → `SystemMeta::tie_break` → コール index 昇順** の順に比べる。`choose_bid` の整列、`ExclusiveIndex`、ナチュラル候補の順位付けは、すべてこの 1 つの比較関数を使う。以前は安定ソートが暗黙に最後の比較を担っていたが、それを明示した。
  - `subtract(base, minus[]) -> HandConstraint`：base ∧ ¬(∪ minus) を計算する。原子レベルの厳密な差集合で、`Atom::negate` の素な連鎖を使う。結果は素な原子の平坦な `Or` で、上限は 48 原子。上限を超えると木 `And([base, Not(Or(minus))])` に退避する（集合としては同じ）。
  - `SystemIR::exclusive(&self) -> &ExclusiveIndex`：`OnceLock` に入れた派生索引。`compile()` の最後に先行して構築する。直列化から復元した IR や手組みの IR では、初回アクセス時に構築する。`#[serde(skip)]` なので直列化形式と `IR_FORMAT` は変わらない。
- `ExclusiveIndex { keys: Vec<(TrieId /*親*/, u8 /*条件クラス*/, u32 /*グループ*/)>, groups: Vec<ExclusiveGroup> }`
  - 条件クラスは `(opener_pos−1) | we<<2 | they<<3` の 16 通りで、`AuctionTrie::children` が席・バル条件で絞る単位と同じである。したがって 1 グループは `choose_bid` の兄弟集合そのものになる。
  - `ExclusiveGroup { members /* rank 順 */, per_call: Vec<(Call, Vec<ExclusivePiece>)>, complement }`
  - `ExclusivePiece { node, branch, constraint /* 平坦な Or か木 */, summary }`
  - 同じ兄弟集合を持つグループは、(call, node) を rank 順に並べた列をキーに重複除去する。
  - 実測（プロトタイプ A、SAYC）：714 グループ、2770 枝、平坦化できずに木へ退避した枝 3、構築 4.6〜10 ms。

§2.3（双方向一致の契約）に 4、5 を追加
4. **逆方向（締まり）**：`choose_bid` が到達する位置で、h が c の非 Fallback 片（システム片 X_c）に入るなら、`choose_bid` は c を選ぶ。前向き（選んだなら入る）と合わせて、X_c は「方策が c を選ぶ手の集合」に一致する。
5. **鏡像**：各位置 P と各合法コール c について、`interpret` の片の密度 D_c(h) = Σ_i w_i·1[h ∈ C_i] は `call_distribution` の p(c | h) の定数倍に一致する。定数 `exp(log_scale)` はコールごとに `CallInterpretation` に記録する。リテラル（cards/eval）を持つ片は上側近似（sup）でしか表せない場合があり、そのときだけ D ≥ p を許す。ただし「D が p を下回る（under-cover）」ことは常に禁止する。

---------------------------------------------
§5.1 `PolicyParams`（差し替え）
---------------------------------------------
```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolicyParams {
    pub epsilon: f32,          // 一様床 ε。既定 1e-3
    pub deviation: f32,        // システム外（ナチュラル）への逸脱 δ。既定 0.0
    pub legacy_temperature: Option<f32>, // Some(τ) で旧 priority ソフトマックス（比較用、フェーズ 6 評価後に削除）。鏡像の保証は無い
}
impl PolicyParams {
    pub fn system_players() -> Self; // = Default。システムどおりに競る前提（生成オークション）
    pub fn human() -> Self;          // コーパス調整用分割で最尤推定した (ε, δ)。フェーズ 4 の統合で (0.3404, 0.3959) に設定。値は 12-roadmap の実績に記録
}
```

---------------------------------------------
§5.2 手順（変更点のみ）
---------------------------------------------
- 手順 4（整列）は `exclusive::rank_cmp` を使う。最後の比較（コール index 昇順）を明示する。
- 手順 1.4（ナチュラル候補）：候補は `NaturalInference::ranked_candidates` が順位順に返す。順位は round(confidence·100) 降順、`tie_break` が `LowestCall`/`HighestCall` ならそれに従い、最後にコール index 昇順。
- 手順 3（暗黙パス）：ナチュラル分岐にも適用する。`ImplicitPass::Complement` で、ナチュラル候補のどれにも合わず Pass が合法なら、`Pass`（priority `i16::MIN + 1`、`source = ImplicitPass`）を合成する。これで、システム外の位置での強制 Pass の穴（gap）がなくなる（プロトタイプ C：1e5 局面で `no_candidate` 234 → 28、gap 起因の違反 111 → 0）。
- 同じコールを持つ候補が複数あっても（Exact 辺と Class 辺の両方から到達する場合など）、方策の選択は「最初に満たした候補のコール」であり、質量を足し合わせることはしない。

---------------------------------------------
§6.1 `call_distribution`（差し替え）
---------------------------------------------
位置 P（接頭辞 `auction[..j]`、手番 s）の合法コールを L（n = |L|）とする。

- s_P(h)：P がシステム内（厳密一致、または寛容照合の最初の完全一致の位置に、合法な子が 1 つ以上ある）なら `choose_bid` のシステム選択。候補が無ければ ⊥。
- m_P(h)：ナチュラル方策の選択。`ranked_candidates` を順に見て最初に満たしたもの。無ければナチュラルの暗黙 Pass。それも無ければ ⊥。
- S(c|h) = 1[s_P(h) = c]。ただし s_P(h) = ⊥ なら 1/n。
- M(c|h) = 1[m_P(h) = c]。ただし m_P(h) = ⊥ なら 1/n。
- π(c|h)：システム内の位置では (1−δ)·S + δ·M、システム外の位置では M。
- **p(c|h) = (1−ε)·π(c|h) + ε/n**

性質：
1. δ < 1/2 なら argmax_c p = `choose_bid` の選択。これは τ に依存しない構造的な等式で、`tests/policy.rs` の 10^5 局面テストはそのまま 100% になる。
2. 同じ優先度どうしでの質量の分け合い（旧 shared 25〜48%）は起きない。
3. δ = 0 のときは、システム内の位置で m_P を評価しない（計算不要）。
4. `legacy_temperature = Some(τ)` のときは旧式（priority/τ の logsumexp → softmax → ε 床）を返す。これは比較評価専用で、`interpret` の鏡像はこの場合を保証しない。

旧定義（τ = 1 の priority ソフトマックス）を捨てる理由は 13-decisions D18 に書く。要点は 2 つある。BML の priority は整列のための小さな整数で、対数オッズとして較正されていない。そして、1NT と 1C の両方を満たす手が 27% で 1C を開くといった分布は、システムの意味にも `replay` にも一致しない。

---------------------------------------------
§4.1 Step A（差し替え：方策鏡像）
---------------------------------------------
各コール j（位置 P、観測コール c、席 s）について、次の「片」を作る。重みは生の値で、`log_scale = ln Σ raw` を記録してから合計 1 に正規化する。したがって p(c|h) = exp(log_scale)·Σ_i w_i·1[h ∈ C_i] が成り立つ。

| 片 | 集合 | 生の重み | kind |
| --- | --- | --- | --- |
| X_c^(b) | システム排他領域の枝 b（下記） | (1−ε)(1−δ) | Exact / Partial |
| N^sys | P のシステム候補を 1 つも満たさない手（`complement`、`ImplicitPass::Never` や明示的な `Pass` 行があるときだけ空でない） | (1−ε)(1−δ)/n | Fallback |
| Y_c | ナチュラル排他領域（下記） | システム内なら (1−ε)δ、システム外なら (1−ε) | Natural |
| N^nat | ナチュラル候補も暗黙 Pass も満たさない手 | システム内なら (1−ε)δ/n、システム外なら (1−ε)/n | Fallback |
| ANY | 全手 | ε/n | Fallback |

δ = 0 のときは Y_c と N^nat を作らない。空の片は落とす。`opts.strict` のときは Fallback の片を落とす（前向き整合性は X_c と Y_c だけで判定する）。

**システム排他領域**：位置 P の兄弟グループ G（`lookup.parent` と条件クラスで索引を引く）について、

  X_c = ∪_{m ∈ G, call(m) = c} ( C_m ∧ ¬ ∪_{m' ∈ G, m' が m より上位, call(m') ≠ c} C_{m'} )

とする。これを、ノードの先頭の `Or` の枝ごとに作る。

- 同じノードの枝どうしは、索引の構築時に素にする（b_i ← b_i ∧ ¬∪_{k<i} b_k）。したがって片は重ならず、密度は正確に (1−ε)(1−δ) になる。素にできない（原子の上限を超えた）ときだけ木にする。
- BML の `{w:}` 枝重みは、提案の密度としては使わない。方策は枝を区別しないからである。枝重みは説明文にだけ残す。
- 上位の兄弟のコールが接頭辞の後で非合法な場合（ワイルドカード部分木など）は、前計算した片を使わず、合法な兄弟だけから実行時に計算し直す（`subtract`、要約で枝刈り）。
- 寛容照合の位置では、`choose_bid` が使う「置換回数最小の最初の完全一致」だけを使う（kind = Partial、重みは Exact と同じ）。他の寛容照合と `ρ^subst` は方策に無いので捨てる。

**方策上選ばれないコール（shadowed）**：X_c も Y_c も空になるコールは、`CallInterpretation.shadowed = true` とし、Fallback の片（N^sys、N^nat、ANY）だけで読む。そのコールがあっても尤度は（N 領域を除いて）手に依らないので、これが方策どおりの読みである。ノード全体で読み直す案は、プロトタイプ C でコーパス ESS を 0.35 から 0.15 に下げたので採らない。説明文には元のノードやナチュラル規則の文を残し、「方策上は選ばれない」と付記する。

**先頭パスと暗黙パス**：ルート（またはその位置）のグループの `complement` をそのまま X_Pass とする。明示的な `Pass` 行があるときは、それを通常の member として扱う。

**ナチュラル排他領域**：ナチュラル候補を順位順に c_1, c_2, … とし、C_k = `infer(classify(prefix.with(c_k)))` とする。

  Y_c = C_c ∧ ¬ ∪_{k が c より上位} C_k

Pass については、ナチュラル暗黙パスとして (Pass 規則 ∨ ¬∪ 他の候補) ∧ ¬上位 とする。計算は `bridge_constraint::grid` の厳密な形 × HCP 集合で行い、次の 2 つの形を持つ。
- 提案形（平坦な Or）：自分の側は sup で取る。上位の候補の側は sub で引く（上側近似にしかならない）。自分の cards/eval リテラルは連言として付ける。
- 所属判定形（木）：`And(C_c, Not(Or(上位)))`。集合として厳密で、尤度（§6.2）に使う。

`infer` はバッチ API `infer_batch`（classify の共通部分を 1 回にまとめ、説明文字列を作らない）で合法コールの分だけ呼ぶ。

**パートナー文脈**：`ctx.partner_constraint` には、パートナーの直前のコールの非 Fallback 片の和の要約（HCP 範囲 + シェイプ）を入れる。`choose_bid` と `interpret` は同じ関数 `partner_context(prefix)` を呼ぶ。ナチュラル排他は要約を変えないので、この経路では計算しない。

§4.2（ε-混合の根拠）に追記
ε-混合は方策の一様床として定義し直す。`eps_exact` / `eps_partial` / `eps_natural` / `lenient_decay` は既定の経路から外し、`InterpretOptions::legacy()` として 1 フェーズだけ残す（ESS の前後比較用）。「信頼度が低いほど ε を大きくする」という役割は、δ（システム外への逸脱）と、方策上選ばれないコールの床に移る。`InterpretOptions` は `InterpretOptions::for_context(&BidContext)` で作る。これで `PolicyParams` と `implicit_pass` を尤度と同じ値から取るので、解釈と尤度がずれることは構造上起きない。

§4.3（「以前の制約を弱める」）の 1 を差し替え
L3 は、各コールに**兄弟の上位候補を除いた排他領域**を使う。祖先の制約は取り込まない（AND は Step B で行う）。以前の「そのノード自身の制約だけ」は廃止する。

---------------------------------------------
§4.4 Step B（変更点）
---------------------------------------------
- 直積と要約による枝刈りは従来どおり行う。片の要約は索引に前計算してあるので、組合せごとに要約を再計算しない。
- 切り詰めの順序：重み w ではなく、見積り質量 w·2^{volume_log2(要約)} の降順で K − 1 = 7 個を残す。提案が成分を w·count に比例して引くので、目標質量の小さい組合せから捨てるためである。
- 全コールの ANY を掛けた受け皿の組合せ（重み Π ε/n の積）は、常に 1 つ残す。これで提案の支持集合が目標の支持集合を覆い、推定が偏らない。
- 重複除去のキーは (node, kind, branch, 片種別) の列にする。

---------------------------------------------
§4.5 に追加
---------------------------------------------
`likelihood(s, h)` は Π_j exp(log_scale_j)·D_j(h) に等しく、方策の尤度そのものになる（定数を含む）。ただしナチュラルの片は所属判定形を使う。

---------------------------------------------
§6.2 `sequence_log_likelihood`（追記）
---------------------------------------------
- 参照実装（各 j で `call_distribution` を呼ぶ）はそのまま残す。
- 高速経路として `AuctionPolicy::new(table, auction, ctx)` を追加する。オークション 1 本につき、各コールの片（所属判定形）と `log_scale` を 1 回だけ組み立て、`log_likelihood(deal) = Σ_j [log_scale_j + ln D_j(h_{s_j})]` を配牌ごとに評価する。システムコールは X_c の所属判定 1 回で済み、兄弟候補を全部評価する必要はない。
- `bridge-sample` の `BiddingLikelihood` はこの高速経路を使う。
- テストで、参照実装との |Δ ln L| ≤ 1e-5 を保証する。

§6.3 `replay`：手順は変えない。7 レベルへの暴走（ESS 原因 (3)）は、06-system §8 のナチュラル・レベル下限で直す。これは方策（目標）の変更なので、解釈の変更とは別の PR で入れ、単独で検証する。

---------------------------------------------
§4.6 性能（追記）
---------------------------------------------
| 項目 | 目標 | 手段 |
| --- | --- | --- |
| `interpret/sayc-12-call-auction`、`interpret/12-call-auction`（δ = 0、既定） | < 10 μs（中央値。loadavg を併記し、負荷が高いときは 3 回の最良値も記録） | システムの片は索引から借用するので割当なし。要約は前計算。Step A/B の内訳を記録する |
| ナチュラル位置 1 コール（コールド） | 追加 ≤ 5 μs | `infer_batch`（プロトタイプの 31 コールの infer は 10.2 μs、その削減が目標） |
| δ > 0（human プリセット） | 12 コール ≤ 40 μs | システム内の位置でもナチュラル候補の列挙が要る。コストは記録する |
| `AuctionPolicy::log_likelihood`（sayc-12） | ≤ 10 μs / 配牌 | 片の所属判定のみ |

`Table` には隠しキャッシュを持たせない（`Table` の構造体リテラルは壊さない）。オークション単位で再利用したい場合は、`InterpretCache` に `AuctionPolicy` を持たせる。

---------------------------------------------
§7 モジュール構成に追加
---------------------------------------------
- `exclusion.rs`：ナチュラル排他領域、実行時の再計算、片の組み立て。
- `auction_policy.rs`：`AuctionPolicy`。
- `bridge-system/src/exclusive.rs`
- `bridge-constraint/src/grid.rs`

---------------------------------------------
§8 テストに追加
---------------------------------------------
| テスト | 場所 | 基準 |
| --- | --- | --- |
| `policy_mirror` | `tests/mirror.rs` | 既定スイートでは 150 位置 × 40 手、`#[ignore]` 版では 2000 × 100。生成位置とコーパス位置の両方で、δ ∈ {0, 0.3} (統合で最尤推定値の `human()` のセルも足した)。under-cover 0、厳密一致 ≥ 99%（リテラルによる over-cover ≤ 1%） |
| `tightness`（プロトタイプ A 由来） | 同上 | δ = 0 の Exact 片について、「内側なのに選ばれない」0、「外側なのに選ばれる」0 |
| `fast_likelihood_matches_reference` | `tests/policy.rs` | 50 オークション × 1000 配牌で \|Δ ln L\| ≤ 1e-5 |
| `forward_consistency` | 既存 | 1e5：gap 起因でない違反 0、gap 起因は ≤ 30（プロトタイプ C では 0）。1e6 は報告する |
| `policy_argmax_matches_choose_bid` | 既存 | 10^5 局面で 100%（両プリセット） |
| `rank_order_shared` | unit | 10^4 位置で、`choose_bid` の `alternatives` の順序が `rank_cmp` の順序と一致 |

---------------------------------------------
§9 未決事項（差し替え分）
---------------------------------------------
1. リテラルを持つ上位候補の差し引き：現在は sub による上側近似。over-cover の率を見て、厳密な DNF に切り替えるかどうかを決める。
2. δ を相手と味方で分けるか、位置の種類（競り合い・オープニング）で分けるか。
3. `legacy_temperature` を削除する時期：フェーズ 6 のリード評価で hard 方策と比較した後。
4. 方策上選ばれない枝（ShadowedBranch lint）を、SAYC の側で消すか残すか。

---------------------------------------------
プロトタイプの実測（§4 末尾に記録、または 09 §10.2 から参照）
---------------------------------------------
50 オークションの ESS スイートでの ESS/n の中央値（全体 / 生成 / コーパス）：
- 基準：0.036 / 0.040 / 0.030
- A（コンパイル時の排他 + rank 方策 τ = 0.2、ε の較正なし）：0.086 / 0.127 / 0.084
- C（実行時の排他 + τ = 0.1・tie_gap 5・ε 0.003・レベル下限）：0.301 / 0.298 / 0.350。これに B の提案側の変更を足すと 0.327、さらに残差棄却を足すと 0.608（試行あたり 0.287）
- B（方策鏡像、τ = 1）：0.426 / 0.503 / 0.377。残差棄却を足すと 0.906（試行あたり 0.371、受理率の中央値 0.52）

教訓：排他（締まり）は必要だが、それだけでは足りない。片の重みを方策の密度に較正して初めて、件数比例の成分抽選が q ∝ L を与える。

==================================================================
06-system.md への注記
==================================================================
- §5（IR）：`SystemIR` に派生索引 `exclusive`（`OnceLock`、`#[serde(skip)]`）を置く。`compile()` の最後で構築する（SAYC で 5〜10 ms）。直列化形式と `IR_FORMAT` は変えない。プロトタイプ A のように直列化すると、postcard の SAYC IR が 825 KB から 1.24 MB（1.51 倍）になり wasm の転送量に効くので、直列化しない。
- §4.3（暗黙パス）：補集合は、兄弟グループ × 条件クラスごとに平坦な素な原子の Or として前計算する（SAYC で 711 グループが平坦、3 グループが木）。
- lint に次を追加する。
  - `ShadowedBranch`（Warning）：上位の兄弟に全域を覆われ、`choose_bid` が決して選ばない枝。SAYC ではプロトタイプの時点で 246 枝。
  - `OverlappingBranches`（Info）：同じノードの枝どうしが重なり、素化した枝。
- §8（ナチュラル推定）
  - `ranked_candidates(auction, owner)`：順位は round(confidence·100) 降順 → `tie_break`（LowestCall/HighestCall）→ コール index 昇順。`choose_bid` のナチュラル分岐と `interpret` のナチュラル排他は、どちらもこの順を使う。
  - `infer_batch(base_ctx, calls)`：`classify` の共通部分を 1 回だけ計算し、説明文字列を作らない。結果は `infer` を 1 コールずつ呼んだものと同一でなければならない。
  - **レベル下限**：3 レベル以上のナチュラル継続ビッドで、自分かパートナーが既に行動していて、`partner_constraint` が分かっている場合に、自分の HCP ≥ combined(level, NT) − パートナーの最小 HCP を課す。combined は、スーツが 3→18、4→22、5→26、6→31、7→35、NT が 3→24、4→28、5→30、6→32、7→36（プロトタイプ C の値）。この表は `NaturalParams.level_floor` として持ち、4.6 で調整する。パートナーが無言のまま自分が初めて行動する場合と、`partner_constraint` が無い場合は、下限を課さない。プロトタイプ C では、2000 生成配牌の最終コントラクトのうち 7 レベルが 1299 から 2 になった。
  - ナチュラル暗黙パス（07 §5.2）。
  - 調整手順（4.6）：コーパスを調整用と評価用に分割し、調整用分割で 3 測定と「実配牌での方策一致率」を最大化する。評価用分割で報告する。ESS スイートでは調整しない。

==================================================================
05-constraint.md への注記
==================================================================
`grid.rs`：`HcpShapeGrid([ShapeSet; 38])` は、560 形 × HCP 0..=37 の厳密な集合である。
- 演算は and / or / diff / not / is_empty / subset で、いずれも 342 語の走査で済む。
- `bounds(c) -> {sub, sup}`：リテラルを含まない And/Or/Not なら厳密（sub = sup）。cards/eval リテラルを含む原子は sub = ∅、sup = 箱。Custom は ∅ / 全体。Not を通ると sub と sup が入れ替わる。
- `to_atoms(template, cap)`：同じ ShapeSet を持つ HCP の連を 1 原子にまとめる。cap を超える場合だけ、広げる方向に併合する。
- 用途：ナチュラル排他と、実行時の再計算。

==================================================================
09-sample.md への注記
==================================================================
- §3.1：`ctx.bidding` が `Some` のとき、`ln L` は `AuctionPolicy::log_likelihood` で求める。値は `sequence_log_likelihood` と同一である。方策は新しい §6.1 のもので、生成オークションには `system_players()`、コーパスとリードアドバイザには `human()` を使う。どちらのプリセットを使うかは評価の前に固定する。
- §6.1 点 4：件数重み付け（v_i ∝ w_i·cnt_i）は正しいので残す。解釈の片の重みが方策の密度であれば、q(h) ∝ Σ w_i·1[h ∈ C_i] = L の席因子になる。プロトタイプ A の「件数重み付けが ANY を引きすぎる」という診断は誤りで、原因は ε（0.02 / 0.30）が方策の床 ε/n（≈ 3e-5）に較正されていなかったことにある。§6.1 点 1 の切り詰めは、07 §4.4 の質量順 + 受け皿の保持に合わせる。
- §6.4 (c)：`coarsen` は、リテラルを含まない原子とその Or を**そのまま残す**（プロトタイプ B の is_literal_free_atoms）。粗くするのは cards/eval を含む項だけにする。
- 残差棄却（新設、既定の採否は計測で決める）：最後の席の手 h について、受理確率 a = D_res(h) / max D で受理し、`log_prob` に ln a を加える。棄却は配牌全体に対して行うので、受理の正規化定数は配牌に依らず、自己正規化の下で偏らない。試行予算は既定 20n とし、予算を使い切ったケースは `SampleReport` に記録する。既定で有効にするのは、「有効サンプル 1 つあたりの壁時計時間」が無効時を下回る場合に限る。
- §9：ESS/n に加えて、**試行あたり ESS**（ESS / 提案総数）、受理率、壁時計時間、そのときの loadavg を `ess_report.json` に出す。生成ケースは固定フィクスチャ `tests/data/ess_cases.txt`（オークション文字列と真の配牌）に凍結する。方策を変えても生成ケース集合は変わらない。凍結は、レベル下限と SAYC 追加が入った後に 1 回行う。コーパスのケースは評価用分割から取る。
- §10.2：上のプロトタイプ表と、原因 (1)(2)(4) が解消したこと（生成の Exact で off_inside 3125 → 0、shared 18.7k → 0）を記録する。残る差は提案側（中間席の粗化、残差席、切り詰め）にある。

==================================================================
11-testing.md への注記
==================================================================
- §2：`policy_mirror` と `tightness` を追加する。1e5 の数値を記録する：基準は gap 起因 111・それ以外 0。A は同じ。B は 119・0。C は 0・0。
- §3（再現率の再定義）
  - (i) **生成**：SAYC の生成フィクスチャ 100 本について、`ConstraintProposal` + 方策尤度で重み付けした「replay == auction」の率。見出しの数値はその中央値で、完了条件は ≥ 0.6。
  - (ii) **コーパス**：SAYC 再現可能部分集合（真の配牌を replay するとコーパスのオークションに一致するもの）の中央値と件数。
  - (iii) 旧定義（コーパス 500 本、一様提案）も継続性のために報告する。
  - (iv) コール単位の「実配牌での方策一致率」（システム位置とナチュラル位置を分けて）。
- §9：ベンチ `interpret/*` の Step A と Step B の内訳、`auction_policy/log_likelihood`、ナチュラル位置の多いベンチ（コーパスの競り合いオークション）を追加する。
- 評価データの管理（新節）
  - 固定フィクスチャ。
  - コーパスの分割規則：`corpus_auctions` の列挙順で、偶数番目を調整用、奇数番目を評価用とする。
  - パラメータ（δ、ε、NaturalParams、レベル下限）は調整用分割で最尤推定・一致率によって決める。ESS では決めない。

==================================================================
13-decisions.md への注記
==================================================================
- **D18 方策は「決定的なシステム選択 + ナチュラル逸脱 δ + 一様床 ε」**
  - 退けた案：τ = 1 の priority ソフトマックス（較正されていない整数を対数オッズとして扱う。shared の分割は replay と矛盾する）、τ = 0.1〜0.2 の rank／priority ソフトマックス（2 位の候補に e^{−1/τ} の質量が残るので、鏡像が複雑になるか under-cover する）、tie_gap（パラメータが 2 つになり、ESS スイートで調整されていた）。
  - 帰結：人間のオークションは δ > 0 のプリセットで読む。δ = 0 では、システム外のコールは情報を持たない。
- **D19 解釈は方策の鏡像。システムの排他領域は派生索引として前計算する**
  - 退けた案：実行時だけの排他（Table に無界のキャッシュが要り、Table の API も壊れる。コールドで 134〜276 μs）、IR への直列化（1.51 倍に膨らむ）、τ = 1 のままの完全な鏡像（プロトタイプ B。SAYC で 65〜205 μs かかり、K = 8 が拘束になる）。
  - 帰結：片の重みは PolicyParams から導く。`InterpretOptions::for_context` を使う。
- **D20 評価手順**：固定フィクスチャ、調整用と評価用の分割、ESS は試行あたりの値も報告、完了条件は「整合したモデルで測る集合」と「コーパス」の両方で報告する（12-roadmap §8）。
- **D15 の改訂**：ε は方策の床 ε/n に一本化する。eps_exact / eps_partial / eps_natural と lenient_decay は既定の経路から退役させる。
- **D11 の追記**：K = 8 は維持する。切り詰めは質量順で行い、受け皿を必ず残す。

==================================================================
12-roadmap.md フェーズ 4 のタスク追加（完了基準の改訂は criteria_changes を参照）
==================================================================
| id | PR の内容 | 証明 / 完了基準 |
| --- | --- | --- |
| 4.7 | 方策鏡像：`ExclusiveIndex`、rank 方策、片の較正、`AuctionPolicy` | `policy_mirror` の under-cover 0、`tightness` 0/0、argmax 100%、interpret 12 コール < 10 μs |
| 4.8 | replay の暴走修正（ナチュラル・レベル下限） | 2000 生成配牌で 7 レベル ≤ 1%、`sayc_content` 通過 |
| 4.9 | 評価基盤：固定フィクスチャ、コーパス分割、`human()` プリセットの最尤推定 | `coverage_report.json` に (ε̂, δ̂) が出る |

## 決定事項（Decisions）

- D18 policy target: p(c|h) = (1-eps)*[(1-delta)*S + delta*M] + eps/n. S = deterministic system choice (choose_bid; uniform when NoCandidate), M = deterministic natural choice (ranked natural candidates plus a natural implicit Pass). The priority softmax at tau=1 is retired; it survives only as PolicyParams.legacy_temperature for the phase-6 comparison. There is no tie_gap and no rank temperature. argmax == choose_bid holds structurally for delta < 0.5.
- Two pre-registered presets. PolicyParams::system_players() (eps 1e-3, delta 0) is used for SAYC-generated auctions. PolicyParams::human() takes (eps, delta) fitted by maximum likelihood on the corpus tune split, from true-deal agreement between human calls and choose_bid/natural. It is used for corpus auctions and the lead advisor. With delta=0, off-system calls at on-system positions carry no information, which is exactly what the model says. With delta>0 they are read naturally, which fixes the 97/100 corpus auctions that leave the system.
- D19 interpretation = calibrated policy mirror. Per call the pieces are X_c^(b) (system exclusive region, per branch, disjointified) with raw weight (1-eps)(1-delta); N_sys (no system candidate) with (1-eps)(1-delta)/n; Y_c (natural exclusive region) with (1-eps)*delta, or (1-eps) off-system; N_nat with (1-eps)*delta/n; ANY with eps/n. CallInterpretation records log_scale, so p(c|h) = exp(log_scale) * sum_i w_i*1[h in C_i] holds exactly. With literals the representation only over-covers; under-cover is never allowed. This replaces the eps_exact/partial/natural and lenient_decay weights; InterpretOptions::legacy() is kept for one phase for before/after comparisons.
- X_c is the set of hands whose first satisfied member in rank order has call c: X_c = union over members m with call c of (C_m AND NOT union of higher-ranked members with a different call). Same-call duplicates (Exact plus Class edges) are unioned, not excluded. BML {w:} branch weights are no longer proposal densities; they stay for explanations only.
- One rank comparator, bridge_system::exclusive::rank_cmp: priority desc, then tie_break, then call index asc. It is used by choose_bid's sort, the index, and the natural candidate ranking (round(confidence*100) desc, then LowestCall/HighestCall, then call index).
- The system exclusion is precomputed by bridge-system as a derived ExclusiveIndex (A's algorithm: exact atom-level subtraction, 48-atom cap with a tree fallback, grouping by (parent TrieId, 16 seat/vul classes) and dedup). It is built eagerly at the end of compile(). It is stored in a OnceLock that is #[serde(skip)] and built lazily for deserialized or hand-built IRs. It is not serialized: IR_FORMAT is unchanged and there is no 1.51x postcard growth. Lookup.parent is added to the trie. Positions where a higher-ranked sibling is illegal recompute from the legal siblings at run time.
- Natural exclusion runs at run time on an exact shape x HCP grid (bridge-constraint grid.rs, [ShapeSet;38], sub/sup bounds as in B, own literals carried as a conjunct). It keeps two forms: a flat over-covering form for the proposal and an exact tree form for membership and likelihood. NaturalInference::infer_batch classifies the shared history once and skips explanation strings.
- Calls the policy never makes at a position are read as Fallback-only (plus the N regions) and flagged CallInterpretation.shadowed. Rationale: under the policy their likelihood is flat in the hand; C measured that the alternative, bare-node reading, drops corpus ESS from 0.35 to 0.15. A new ShadowedBranch lint (Warning) reports the 246 SAYC branches.
- choose_bid's natural branch synthesises an implicit Pass under ImplicitPass::Complement (from C). This removes the forced-Pass gaps: no_candidate 234 -> 28 and gap-induced violations 111 -> 0 at 1e5.
- Step B keeps K=8, but truncates by estimated mass w*2^volume_log2 rather than by w. It always retains the all-ANY catch-all combo, so the proposal support covers the target support.
- Fast likelihood: AuctionPolicy::new(table, auction, ctx) precomputes each call's pieces and log_scale once per auction; log_likelihood(deal) is membership tests only. It is asserted equal to the reference sequence_log_likelihood (|d ln L| <= 1e-5), and bridge-sample's BiddingLikelihood uses it.
- InterpretOptions::for_context(&BidContext) derives PolicyParams and implicit_pass from the same BidContext used by the likelihood, so the mirror and the target cannot drift. Table gets no hidden caches and no private field, so the Table struct literal still compiles.
- Replay escalation (root cause 3) is fixed by C's natural level floor, moved into NaturalParams.level_floor. It lands as a separate PR, because it changes the target and the generated auctions, and is validated on its own (7-level rate, sayc_content, natural agreement on the eval split).
- Sampler: literal-free coarsen from B (keep literal-free atom disjunctions unchanged). Residual rejection from B, with an attempt budget (default 20n) and reporting of ESS per attempt, acceptance and wall time. It is enabled by default only if wall time per effective sample improves.
- D20 evaluation protocol. The ESS suite's generated cases are frozen into a fixture after the level floor and the SAYC additions land. The corpus is split by enumeration index (even = tune, odd = eval). delta, eps, NaturalParams and the level floor are fitted on the tune split by likelihood or agreement, never on ESS. Every criterion is reported for both the well-specified set (SAYC-generated) and the corpus.

## レーン（Lanes）

### S: system index, grid, natural (bridge-system / bridge-constraint)

- files: crates/bridge-constraint/src/grid.rs (new), crates/bridge-constraint/src/lib.rs, crates/bridge-constraint/tests/grid.rs (new); crates/bridge-system/src/exclusive.rs (new), crates/bridge-system/src/trie.rs, crates/bridge-system/src/ir.rs, crates/bridge-system/src/lib.rs, crates/bridge-system/src/compile/mod.rs, crates/bridge-system/src/lint.rs, crates/bridge-system/src/natural.rs, crates/bridge-system/tests/exclusive.rs (new), crates/bridge-system/tests/natural_*.rs, crates/bridge-system/tests/compile_time.rs; crates/bridge-bidding/tests/natural_metrics.rs; docs/design/06-system.md, docs/design/05-constraint.md
- depends_on: none. Lanes B and P depend on commit 0 of this lane.

Commit 0, within the first half day, is an API-first commit that lanes B and P build against. It contains signatures and working stubs for: Lookup.parent; SystemIR::exclusive() -> &ExclusiveIndex (OnceLock, serde skip); exclusive::{rank_cmp, subtract, subtract_tree}; the ExclusiveGroup/ExclusivePiece types including a per-call piece list and a precomputed Summary per piece; grid::HcpShapeGrid with bounds/to_atoms; and NaturalInference::{ranked_candidates, infer_batch}. Then:
(1) Port A's exclusive.rs (wip/p4d-A-compile 57bd61d). Add per-call union semantics for same-call members and disjointification of branches within a node. The eager build goes at the end of compile(); do NOT serialize and do NOT bump IR_FORMAT.
(2) Add ShadowedBranch (Warning) and OverlappingBranches (Info) lints.
(3) Write grid.rs, unifying C's exclusion.rs Grid and B's grid.rs into one exact module with sub/sup bounds.
(4) In natural.rs: ranked_candidates in rank order; infer_batch (classify once, no explanation strings); C's level floor as NaturalParams.level_floor, including the natural implicit-Pass rule text.
(5) Phase 4.6: tune NaturalParams (confidence values act as ranks now; level-floor table) on the corpus tune split using natural_metrics' three measurements plus true-deal agreement; report on the eval split.
(6) Update the 06-system.md sections (§5 IR, §4.3, lint, §8) and 05-constraint.md (grid).

### B: policy mirror in bridge-bidding

- files: crates/bridge-bidding/src/lib.rs, crates/bridge-bidding/src/interpret.rs, crates/bridge-bidding/src/choose.rs, crates/bridge-bidding/src/policy.rs, crates/bridge-bidding/src/replay.rs, crates/bridge-bidding/src/cache.rs, crates/bridge-bidding/src/exclusion.rs (new), crates/bridge-bidding/src/auction_policy.rs (new), crates/bridge-bidding/tests/{consistency.rs, policy.rs, unit.rs, review_regressions.rs, replay.rs, mirror.rs (new), common/mod.rs}, crates/bridge-bidding/benches/interpret.rs; docs/design/07-bidding.md, docs/design/13-decisions.md
- depends_on: Lane S commit 0 (API). It can start immediately against the runtime-recompute path; switch to the index when S step (1) lands.

(1) choose.rs: B's enumerate_candidates refactor, so choose_bid, call_distribution and interpret share one hand-independent candidate list. Sort with exclusive::rank_cmp. Add the natural-branch implicit Pass (from C). Add a shared partner_context(prefix) that returns the summary of partner's last call's non-Fallback pieces.
(2) policy.rs: the new PolicyParams {epsilon, deviation, legacy_temperature}, the system_players()/human() presets (the human values come from lane D's fit at integration; use a placeholder until then)（フェーズ 4 の統合で ε 0.3404、δ 0.3959 に設定済み。12-roadmap）, and the new call_distribution formula.
(3) interpret.rs + exclusion.rs: Step A emits the calibrated pieces X/N_sys/Y/N_nat/ANY with log_scale and the shadowed flag. X comes from the index via lookup.parent, with a run-time recompute when a higher-ranked sibling is illegal. For lenient positions use only the first full lenient match. Natural exclusion uses grid + infer_batch in two forms: flat for the proposal, tree for membership. Add InterpretOptions::for_context and legacy().
(4) Step B: precomputed summaries, mass-ordered truncation, and an always-kept catch-all combo.
(5) auction_policy.rs: AuctionPolicy with a fast log_likelihood; keep sequence_log_likelihood as the reference.
(6) Tests: mirror.rs (policy_mirror, plus tightness ported from A), fast==reference, rank_order_shared, and updates to policy/consistency.
(7) Speed work to reach interpret 12-call < 10 us. Record the Step A/B split and loadavg.
(8) Write 07-bidding.md from the design text and 13-decisions D18/D19/D20 with the D15/D11 amendments.

### P: sampler, reproduction and lead evaluation

- files: crates/bridge-sample/src/* (constraint_proposal.rs, lib.rs, proposal.rs, report.rs, weights.rs, uniform.rs), crates/bridge-sample/tests/* (ess_suite.rs, ess.rs, log_prob.rs, tracing_info.rs, support/mod.rs, data/ess_cases.txt new), crates/bridge-sample/benches/deals.rs; crates/bridge-bidding/tests/reproduction.rs; crates/bridge-lead/src/*, crates/bridge-lead/tests/*; docs/design/09-sample.md, docs/design/11-testing.md, docs/design/14-lead.md
- depends_on: Lane S commit 0 for the types. Code can be written in parallel; the final numbers require lanes S and B to be integrated, and the fixture freeze requires lane D's SAYC changes.

(1) Port B's literal-free coarsen and residual rejection (wip/p4d-B-mixture b2743b9, constraint_proposal.rs only), adding an attempt budget (default 20n) and budget-exhaustion reporting in SampleReport. Align prepare's truncation with mass ordering and keep the catch-all.
(2) BiddingLikelihood uses AuctionPolicy::log_likelihood; until lane B lands it falls back to sequence_log_likelihood.
(3) ESS suite:
- report ESS/n, ESS per attempt, acceptance, wall time and loadavg;
- use presets per source (generated -> system_players, corpus -> human);
- corpus cases come from the eval split;
- add a fixture writer and freeze tests/data/ess_cases.txt at integration, after lanes S (level floor) and D (SAYC rows) have landed;
- add a tuning mode with a different seed set.
(4) Rewrite the reproduction harness:
- (i) generated fixture of 100 auctions, with ConstraintProposal and the policy likelihood;
- (ii) the corpus SAYC-reproducible subset (true deal replays to the recorded auction);
- (iii) the legacy definition, kept for continuity;
- (iv) per-call true-deal agreement.
(5) Lead advisor: use InterpretOptions::for_context with the human preset. Evaluate top-1 and top-3 against baselines (a)/(b), comparing hard against legacy_temperature=1. Give the flaky dds_smoke a 1e-9 FP tolerance.
(6) Decide whether residual rejection is on by default using time per effective sample.
(7) Update 09-sample.md (§3.1, §6.1, §6.4c, residual rejection, §9, §10.2 with the prototype table), 11-testing.md (§2, §3, §9, the evaluation-data section) and 14-lead.md.

### D: SAYC data, coverage report, criteria

- files: xtask/src/main.rs, xtask/src/coverage.rs (new), xtask/Cargo.toml; systems/sayc/*.bml, systems/sayc/NOTES.md; crates/bridge-bidding/tests/sayc_content.rs; crates/bridge-system/tests/sayc.rs; docs/design/12-roadmap.md
- depends_on: No code dependency: it uses the public interpret/replay/choose_bid API. Re-run the report after lanes S and B are integrated. It provides the human-preset values to lane B at integration.

(1) Phase 4.1, `cargo xtask coverage`. It writes target/coverage_report.json with:
- corpus: auction all-Exact rate and call-level kinds (Exact/Partial/Natural/shadowed), for all auctions, the eval split, and the SAYC-compatible-opening subset (the true opener's hand is in X of the recorded opening);
- counts of seats with empty strict support and of sampler EmptySupport;
- the resolve_lenient usage rate;
- true-deal policy agreement (system positions / natural positions);
- the MLE fit of (eps, delta) on the tune split: count each human call as sys, nat or other and maximise the policy log-likelihood over a grid;
- generated: 1000 fixed-seed random-deal replays with natural completion, reporting the all-system rate, the NoCandidate top 50, the top natural-completion positions, and the final-contract level histogram.
(2) Phases 4.2-4.4: SAYC rows guided by the NoCandidate and natural-completion tops. Start with the phase-3 tops: 1D-(3C) responder, P-P-1D-(1H) responder, 1C-(1H) responder. Then do responses, rebids and competition. New rows must not be shadowed.
(3) Keep sayc_content green, including the level-floor case [1C 1D 1S 3D] -> 3S.
(4) Write the phase 4/5/6 criteria revision and the new tasks 4.7-4.9 into 12-roadmap.md, and record the baseline and final numbers there.

## 受け入れ基準（Acceptance）

- Common gates for every lane: cargo clippy --workspace --exclude bridge-dds --all-targets --all-features -D warnings is clean; cargo test --workspace --exclude bridge-dds --all-features passes; the wasm32 check passes; MSRV 1.85 (no let-chains); no new dependencies; no unsafe outside bridge-dds; every timing is recorded together with sysctl -n vm.loadavg.
- S, exclusive index: over 1e5 random (position, hand) pairs on SAYC, membership in X_c equals 'the first satisfied member in rank_cmp order has call c', with 0 mismatches. Complements match the same check with 0 mismatches. Pieces of one node are pairwise disjoint, with 0 overlaps over 1e5 hands. Tree fallback covers <= 1% of branches (A: 3 of 2770). Index build <= 15 ms release. SAYC compile time grows by <= 15 ms. Postcard SAYC IR bytes are unchanged (825,324) and IR_FORMAT is unchanged. The ShadowedBranch lint count is reported (about 246 expected).
- S, grid: over 200 constraints x 1e5 hands, literal-free constraints show 0 mismatches against satisfies(), and for literal constraints sub is a subset of C and C a subset of sup on every sample. infer_batch returns constraints identical to per-call infer at every natural position of 1000 generated and 500 corpus auctions, and runs in <= 4 us for 31 legal calls (1NT P 2C; base 12 us).
- S, natural: level floor checked on 2000 fixed-seed generated deals. 7-level final contracts are <= 1% (the base without the floor had 1299/2000) and 6+ level contracts <= 5%. sayc_content passes. Phase 4.6: on the corpus eval split, none of the three natural_metrics measurements drops by more than 0.01, and decision-point agreement rises by >= 0.02 over 0.329 (or the lane documents why not).
- B, mirror: policy_mirror (default suite 150x40; ignored 2000x100; SAYC generated and corpus positions; delta in {0, 0.3}) shows under-cover 0 and exact >= 99%. tightness on Exact pieces at delta=0 gives inside-and-not-picked 0 and outside-and-picked 0 (A reached exactly this). The fast likelihood equals the reference sequence_log_likelihood with max |d ln L| <= 1e-5 over 50 auctions x 1000 deals.
- B, consistency and policy: 1e5 release forward consistency (seed 0x5a1c0002) has 0 non-gap violations and <= 30 gap-induced (C reached 0; base 111). The 1e6 run is reported. policy_argmax_matches_choose_bid passes at 100% on 1e5 positions under both presets. rank_order_shared passes on 1e4 positions.
- B, speed (release criterion, median; if loadavg > 4, also report the best of 3 runs): interpret/sayc-12-call-auction and interpret/12-call-auction < 10 us (base 17.6 / 11.3). interpret/sayc-1nt-auction <= 10 us, sayc-competitive <= 8 us, 12-call-auction-realistic <= 12 us. A new natural-heavy corpus bench adds <= 5 us per natural call when cold. With the human preset, sayc-12 <= 40 us. AuctionPolicy::log_likelihood <= 10 us per deal on sayc-12 (the sampler's reference path was about 92 us per deal on Stayman 3NT).
- B, early signal before lane P lands: the ESS suite run with the unchanged proposal gives a median ESS/n >= 0.25 (C reached 0.30 without calibration).
- P, sampler correctness: small-pool chi-square and log_prob exactness tests pass with coarsen and residual rejection on. A small-pool unbiasedness test shows weighted posterior estimates matching the exact enumeration (chi-square p > 0.01). The attempt budget is honoured, and budget exhaustion is reported.
- P, ESS (frozen 50-case suite; generated cases use system_players, corpus cases use human on the eval split; n=1000): median ESS/n >= 0.5 overall and >= 0.5 on the generated set; corpus >= 0.4. ESS per attempt, acceptance median/min and wall time are all reported (target: ESS per attempt >= 0.35). If residual rejection is on, <= 2 of 50 cases may exhaust the budget and the suite must take <= 2x the wall time of the no-rejection run; the suite finishes in <= 60 s release.
- P, throughput: the bridge-sample deals bench reaches >= 1e4 deals/s/core on all three real-SAYC cases, including Stayman 3NT (was 8.1K), using the fast likelihood.
- P, reproduction: on the generated fixture (100 auctions), the median weighted reproduction is >= 0.6. For the corpus SAYC-reproducible subset, the count and median are reported, with a target median >= 0.6. The legacy definition and per-call true-deal agreement are reported.
- P, lead advisor (100 corpus boards, eval split where possible, n=100, human preset): top-3 >= 0.90 and top-1 >= baseline (a) (0.808). Median ESS is reported (target >= 20; it was 4). hard vs legacy_temperature=1 is compared; if hard is worse on top-1 by more than 0.02, D18 is reopened.
- D, coverage: cargo xtask coverage runs in <= 120 s release and writes every listed field, with a baseline recorded before any SAYC change. On generated SAYC auctions (1000 fixed-seed replays), >= 80% are all-system (no natural completion, no gap). The phase-3 NoCandidate tops (1D-(3C) responder 36, P-P-1D-(1H) responder 31, 1C-(1H) responder 25, measured per 1e6) each drop by >= 80%. New rows add 0 ShadowedBranch lints, Error lints stay at 0, and the forward-consistency non-gap count stays at 0. On the corpus SAYC-compatible-opening subset, call-level system resolution (Exact or Partial) reaches >= 80% (target), and the all-Exact rates for all, eval and subset are reported. The resolve_lenient usage rate falls below the baseline. The (eps, delta) MLE is reported together with its log-likelihood curve.

## 完了基準の改訂案（Criteria changes）

WHY CHANGE: the corpus (2019 world championships and BBO vugraph) was played mostly in strong-club and 2/1 systems with their own conventions, but it is interpreted and replayed as SAYC. That is model misspecification. 97 of 100 corpus auctions contain a natural call at an on-system position, and the phase-3 reproduction on the corpus is 0.0 for all three prototypes, whatever the interpretation does (B reaches ESS >= 30 on 92/100 and still gets median 0). A corpus-only threshold therefore measures the corpus's system mix, not SAYC's coverage or the sampler's quality. The ESS criterion also becomes ambiguous once rejection is allowed: B's ESS/n is 0.906, but its ESS per attempt is 0.371. Finally, the generated ESS cases changed whenever the policy changed (C's level floor), and several parameters were tuned on the evaluation suite itself. The spirit is kept: the same thresholds (80%, 0.6, 0.5), applied to a well-specified set, with a defined corpus counterpart, and both always reported.

PHASE 4 (revised):
(a) Coverage:
- [G] >= 80% of 1000 fixed-seed SAYC-generated auctions (random deals, replay with natural completion) are all-system: no natural completion and no gap. This is the coverage the system author controls, and it is not tautological, because natural completion marks the holes.
- [C] On the corpus SAYC-compatible-opening subset (the true opener's hand lies in X of the recorded opening), call-level system resolution (Exact or Partial) is >= 80%.
  - 統合時の注記: [G] と同じく strict 集計に適用する。呼び手のシステムが既定パス (priority ≤ −100 のパス、`{stop}` の行やシステム停止の合成パス) しか出さない位置の Exact / Partial は解決に数えない (`xtask coverage` の `system_resolution_strict_rate`)。素の値も並べて報告する。理由: 停止は際限が無いので、停止の後の人のパスがすべて「解決」になり、作者が書いていないカバレッジが素の値に入る (`12-roadmap.md` フェーズ 4 の停止の節)。
- The auction-level all-Exact rate is reported for the whole corpus, the eval split and the subset, compared against the 4.1 baseline and required not to fall.
- EmptySupport: 0 sampler EmptySupport over the corpus under the default mode; seats with empty strict support are reported.
(b) Reproduction:
- [G] median >= 0.6 on 100 generated auctions, weighted by ConstraintProposal and the policy likelihood.
- [C] median >= 0.6 on the corpus SAYC-reproducible subset (true deal replays to the recorded auction), with the subset size reported.
- The legacy full-corpus uniform-proposal median and per-call true-deal agreement are reported for continuity.
(c) New gates:
- policy_mirror under-cover 0 and exact >= 99%;
- tightness 0/0;
- consistency: 0 non-gap violations;
- interpret 12 calls < 10 us, carried over from phase 3 and still unmet there.

PHASE 5 (revised):
- On the frozen 50-case suite (25 generated with system_players, 25 corpus from the eval split with human; presets pre-registered), median ESS/n >= 0.5 overall and >= 0.5 on the generated set; corpus reported, with a target of >= 0.4.
- ESS per attempt, acceptance and wall time are always reported. If residual rejection is used, the attempt budget is <= 20n, <= 2 of 50 cases may exhaust it, and wall time must be <= 2x the no-rejection run.
- >= 1e4 deals/s/core, now including Stayman 3NT.
- Parameters are fitted only on the tune split and seeds.

PHASE 6: fix the open threshold X as top-3 >= 0.90 AND top-1 >= baseline (a) on the 100-board set, using the human preset. Report hard vs legacy tau=1, and median ESS (target >= 20).

REPORTED FOR BOTH, every time:
- all-Exact / system-resolution rates on generated and corpus (all, eval, subset);
- reproduction: generated, corpus subset, and legacy corpus;
- ESS/n and ESS per attempt, generated and corpus;
- true-deal agreement per call on the corpus;
- the (eps, delta) MLE.
