# Real BML file lint triage (roadmap 3.2-3.4)

Every `Error`-severity lint the compiler produces over `systems/vendor/data` (bml-test, jdh8, gjp), triaged into:

- **(a) our compiler bug** — fixed directly (see `compile/desc/{clause,context,tokens}.rs` and `compile/expand.rs`), each with its own small inline-BML regression test. None remain open; this table lists only what could not be fixed this round.
- **(b) genuine contradiction in the source file itself** — recorded, not fixed (the file is self-contradictory, the vendored fixture is intentionally so, or the author's own auction is illegal under every expansion).
- **(c) design/vocabulary gap** — a real compiler limitation (grammar, Pass 2 context resolution, or vocabulary coverage) that is known and recorded rather than chased further this round; each is a candidate v2 task.

Every row below is also a line in `tests/data/real_expected_errors.txt`, which `tests/compile_real.rs` asserts the actual Error set equals exactly. `tests/compile_real.rs` resolves each lint's `span.file` to the real file it names (not the root file that happened to `#INCLUDE` it), so this table's `file:line` is the actual source location, never a root aggregator's own line count standing in for an included file's. Grouped by root cause; each group states the class and the number of `file:line` occurrences, then lists them.


Total: **64** Error-severity lints, 13 distinct root causes, across 21 files.


By class: (b) genuine source contradiction = 1, (c) design/vocabulary gap = 63.


## Root-cause groups


### 1. [c] Same architectural family as the splinter/agreed-trump-suit fix: the splinter/transfer's own suit and the inferred agreed/target suit collide under a context that fix does not cover (partner's call names no suit, or the position is reached via a trie node shared with another table); recorded as a known Pass-2 context-resolution gap.


25 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C-1M-2M.bml:11` | UnsatisfiableConstraint | MAX, 4M, SPL !d (om) |
| `gjp/common/1C-1M-2M.bml:17` | UnsatisfiableConstraint | MAX, 4!h, SPL !s |
| `gjp/common/1C-1M-2M.bml:20` | UnsatisfiableConstraint | MAX, 4!s, SPL !h |
| `gjp/common/1M-fit.bml:30` | UnsatisfiableConstraint | SPL, stronger than via 3!c |
| `gjp/common/1M-fit.bml:32` | UnsatisfiableConstraint | 4M, SPL in the other major, 13-15 HCP |
| `gjp/common/1M-fit.bml:33` | UnsatisfiableConstraint | 4M, SPL m, 13-15 HCP |
| `gjp/common/1N.bml:90` | UnsatisfiableConstraint | 55MM, MAX, SPL m |
| `gjp/common/1m-(1X).bml:35` | UnsatisfiableConstraint | Transfer to 1!s. At least 4!s. |
| `gjp/common/2C.bml:66` | UnsatisfiableConstraint | weak-two, MAX, good suit, SPL !c |
| `gjp/common/2C.bml:67` | UnsatisfiableConstraint | weak-two, MAX, good suit, SPL !h |
| `gjp/common/2C.bml:68` | UnsatisfiableConstraint | weak-two, MAX, good suit, SPL !s |
| `gjp/common/2C.bml:69` | UnsatisfiableConstraint | weak-two, MAX, good suit, no SPL |
| `gjp/common/2D.bml:21` | UnsatisfiableConstraint | Splinter, MIN/MAX |
| `gjp/common/2N-Puppet.bml:28` | UnsatisfiableConstraint | 5!s-4!h, TRF to 4!s |
| `gjp/common/2N-Puppet.bml:70` | UnsatisfiableConstraint | 6!h, TRF to 4!h |
| `gjp/common/2N-Puppet.bml:72` | UnsatisfiableConstraint | 6!s, TRF to 4!s |
| `gjp/common/2N.bml:64` | UnsatisfiableConstraint | 6!h, TRF to 4!h |
| `gjp/common/2N.bml:66` | UnsatisfiableConstraint | 6!s, TRF to 4!s |
| `jdh8/blue/1D.bml:33` | UnsatisfiableConstraint | F, MIN SPL, 1=!c, 3+!s, 3+!h, 5+!d |
| `jdh8/wj/1C.bml:43` | UnsatisfiableConstraint | SPL, 21--23 HCP, 40(54) |
| `jdh8/wj/1C.bml:44` | UnsatisfiableConstraint | SPL, 21--23 HCP, 04(54) |
| `jdh8/wj/1M.bml:92` | UnsatisfiableConstraint | INV, decline a !d SPL |
| `jdh8/wj/1M.bml:93` | UnsatisfiableConstraint | INV, decline a !h SPL |
| `jdh8/wj/2C.bml:10` | UnsatisfiableConstraint | MAX SPL, 0--1!c, 5+!s, 5+!h |
| `jdh8/wj/2C.bml:11` | UnsatisfiableConstraint | MAX SPL, 0--1!d, 5+!s, 5+!h |

Note: this round's `partner_bid_this_suit`/`agreed_suit_or_partner_last` fix (class a, see below) removed 8 entries this group used to carry (`bml-test/data/example3.bml:27`, `gjp/common/2suitedovercalls.bml:106`, `jdh8/blue/1C.bml:83` and `:120`, `jdh8/blue/1M.bml:73`, `jdh8/common/2NT-UNT.bml:21`, `jdh8/wj/1C.bml:308`, `jdh8/wj/1M.bml:111`) and 21 more were never real second locations at all -- they were the same source Errors as the ones still listed above, double-counted under the root file that `#INCLUDE`s them (`jdh8/blue.bml`, `jdh8/wj.bml`, `jdh8/defense.bml`) at the *included* file's line number, a `tests/compile_real.rs` attribution bug (see below) fixed this round.


### 2. [c] A parenthetical aside describing an alternative call or exception (else/otherwise/also/maybe/opener can have) is not excluded from the row's constraint and is ANDed in as if it were a literal requirement on this hand.


9 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C.bml:82` | UnsatisfiableConstraint | FG, 5+!h (opener can have 3 cards so with 4!h you bid 4SFG first), longer diamonds |
| `gjp/common/1C.bml:87` | UnsatisfiableConstraint | FG, 5+!s (opener can have 3 cards so with 4!s you bid 4SFG first), longer diamonds |
| `gjp/common/1C.bml:147` | UnsatisfiableConstraint | 5M-5m, good suits (else 2!d), FG |
| `gjp/common/1C.bml:174` | UnsatisfiableConstraint | MAX, S/S (also 3M) |
| `gjp/common/1C.bml:191` | UnsatisfiableConstraint | NAT, INV (1!c-2!h shows 5!s-4!h and a weak hand) |
| `gjp/common/1C.bml:216` | UnsatisfiableConstraint | 5M-4m, good suits (else 2!s), S/T |
| `gjp/common/1C.bml:217` | UnsatisfiableConstraint | 5M-5!d, good suits (else 2!s), S/T |
| `gjp/common/1C.bml:218` | UnsatisfiableConstraint | 6M, good suit (else 2!s), S/T |
| `gjp/common/1C.bml:331` | UnsatisfiableConstraint | nothing to bid (like 1C-1D without intervention) but with at least 3!c, opener rebids l... |

Note: this round's NAT explicit-wins fix (class a, see below) removed 3 entries this group used to carry (`gjp/common/1C.bml:54`, `gjp/common/1H-1S.bml:36`, `gjp/common/1M-1N.bml:27`) -- each was really the same NAT-vs-explicit-length bug as the fixed cases, not a genuine parenthetical-aside contradiction.


### 3. [c] Pass-2 context resolution does not yet model this combination of context-dependent words/fragments for this row; recorded as a known gap rather than chased further this round.


8 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C-1M-2M.bml:8` | UnsatisfiableConstraint | MAX, 3M, NAT |
| `gjp/common/1C-1M-2M.bml:10` | UnsatisfiableConstraint | MAX, 4M |
| `gjp/common/1C.bml:38` | UnsatisfiableConstraint | 6!c, 6-9 HCP, expects to win 3NT opposite a strong balanced hand |
| `gjp/common/1C.bml:247` | UnsatisfiableConstraint | 5422, 4M, FG |
| `gjp/common/1C.bml:333` | UnsatisfiableConstraint | as before but with at most 2!c, opener rebids like after 1!c-1!d without further interv... |
| `gjp/common/2N-Puppet.bml:36` | UnsatisfiableConstraint | no 4M, no interest in playing 4!s opposite 5!s-4!h |
| `jdh8/wj/1C.bml:120` | UnsatisfiableConstraint | QUANT INV to 6NT |

Note: this round's NAT explicit-wins fix (class a, see below) removed `gjp/common/1H-1S.bml:46` from this group -- it was the same NAT-vs-explicit-length bug, not a genuine Pass-2 gap.


### 4. [c] Two context-dependent strength words joined by a comma are intended by the author as alternative branches (if spades, invitational; if hearts, forcing / usually X but sometimes Y), but the grammar's comma is a strict AND, so their HCP ranges are incompatibly conjoined.


4 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C-1M-2M.bml:4` | UnsatisfiableConstraint | MIN, usually 4M but sometimes 3M is possible |
| `jdh8/common/2X-Multi-Muiderberg.bml:66` | UnsatisfiableConstraint | P/C, INV in !s, FG in !h |
| `jdh8/defense/2X-NAT.bml:43` | UnsatisfiableConstraint | MIN INV, usually 4=# |
| `jdh8/wj/1D.bml:243` | UnsatisfiableConstraint | PRE 7+!c, INV+ 4+!d, or UNBAL FG |


### 5. [c] The (3)4+ optional-count shorthand (3-or-4-plus cards) is not recognized by the shape/length grammar and is parsed in a way that contradicts the row's own explicit suit-count fragments.


4 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2suitedovercalls.bml:53` | UnsatisfiableConstraint | NF, (3)4+!h, light INV |
| `gjp/common/2suitedovercalls.bml:54` | UnsatisfiableConstraint | NF, (3)4+!s, light INV |
| `gjp/common/2suitedovercalls.bml:80` | UnsatisfiableConstraint | NF, (3)4+!h, light INV |
| `gjp/common/2suitedovercalls.bml:81` | UnsatisfiableConstraint | NF, (3)4+!s, light INV |


### 6. [c] Gambling 3NT cue-bid convention: the call's own suit and the opponents' cue suit resolve to the same bound history variable, so solid 7+ suit (needs AKQ) and without stopper (needs no AKQ) directly contradict in the same suit.


4 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `jdh8/blue/1C.bml:214` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/defense/1X.bml:11` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/wj/1C.bml:440` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/wj/1M.bml:199` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |


### 7. [c] A weak-two augmentation word (MAX / fit) combines with the base weak-two length/HCP range in a way Pass 2 does not model, producing a range that excludes the row's own explicit suit fragment.


3 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2C.bml:59` | UnsatisfiableConstraint | weak-two in !d, fit, MAX |
| `gjp/common/2D.bml:56` | UnsatisfiableConstraint | weak-two in !s, MAX, good suit |
| `gjp/common/2D.bml:57` | UnsatisfiableConstraint | weak-two in !h, MAX, good suit |


### 8. [c] A multi-line (a)/(b)/(c) lettered list of alternative meanings is joined by the grammar as a strict AND across lines instead of as OR across lettered branches.


2 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `jdh8/blue/1D.bml:13` | UnsatisfiableConstraint | F, Multi:\n(a) NAT FG, 4+!c, 0--3!s, 0--3!h\n(b) PRE TRF, 0--7 HCP, 4--5!d\n(c) PRE, 0-... |
| `jdh8/wj.bml:74` | UnsatisfiableConstraint | F, Polish Club:\n(a) 12--14 HCP, 2--4!s, 2--4!h, 2--4!d, 2--4!c\n(b) 11--17 HCP, 5+!c o... |

Note: `jdh8/wj.bml:74` is a genuine root-level row (`wj.bml`'s own opening-bids overview table, lines 66-100, written directly in the aggregator file rather than pulled in through an `#INCLUDE`), not a misattribution -- unlike every other `jdh8/{blue,wj,defense}.bml` entry this triage used to carry.


### 9. [c] "same meaning and development as after X" is a cross-reference aside, a different phrasing than "see X" (handled by the description compiler's see-reference fix) that the vocabulary does not yet recognize as opaque.


2 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2C.bml:70` | UnsatisfiableConstraint | same meaning and development as after 2!c-2!d-3X |
| `gjp/common/2D.bml:59` | UnsatisfiableConstraint | same meaning and development as after 2!d-2!h-3X |


### 10. [c] "<shape> possible after <auction>" is a conditional footnote about a different call sequence, not a literal shape constraint on this row; the vocabulary does not yet recognize this phrasing as opaque (same family as the see-reference handling).


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C.bml:115` | UnsatisfiableConstraint | 12-14 NT. 4333 possible after 1!c-1!h. |


### 11. [b] The auction is 1N-(P)-2C-(D) then opener P then (R): the parentheses mean East redoubles after West (his own partner) doubled -- a player can never redouble his own side's double, so this call is illegal under every expansion. The author evidently meant responder's redouble.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1N.bml:163` | IllegalCall | Stayman again, INV+ |

Reclassified this round from (c) "a known gap in call-legality resolution for a repeated (R) step" to (b): the call is illegal under every expansion regardless of that resolution, since a player redoubling his own side's own double is never legal in the first place (`systems/vendor/data/gjp/common/1N.bml:161-163`: `1N-(P)-2C-(D)` / `P = ...` / `(R) = Stayman again, INV+` -- the `(D)`/`(R)` parentheses put the double and the redouble on the *same* side).


### 12. [c] The file's free-text intro paragraph before the first auction table is parsed as a row with an empty description; an empty description should compile to Atom::ANY (trivially satisfiable) but this row ends up Unsatisfiable instead -- recorded as a parser-classification gap rather than fixed this round.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1m-2m.bml:3` | UnsatisfiableConstraint | |


### 13. [c] The qualifier before the strength word (light before INV) is not recognized by the vocabulary, so the plain INV range is used as if unqualified, conflicting with the row's own explicit suit-length fragment.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2suitedovercalls.bml:102` | UnsatisfiableConstraint | light INV, !s |


## Fixed this round (class a: compiler bugs, not source contradictions)

These used to appear above (or would have, had `tests/compile_real.rs` attributed them correctly); each is now fixed directly, with its own regression test, and produces no Error any more.

- **`partner_bid_this_suit` (`compile/expand.rs`) / `agreed_suit_or_partner_last` (`compile/desc/context.rs`)**: a partner call agreed the suit it named merely for sharing a strain with the row's own later call, even when partner's own constraint never actually showed length there (e.g. a two-suiter like `1C-1D-2S = 16+, 5+!c and 4+!h`, which is clubs-and-hearts, not spades). A splinter or support row after such a call then ANDed a 3+/4+ length requirement in that suit against its own shortness claim, an unsatisfiable contradiction. Fixed by requiring partner's call to be non-artificial *and* to show at least 3 cards in the suit before agreeing it (`docs/design/06-system.md` §7.5's `partner_len_min`). Regression tests: `compile::expand::tests::splinter_does_not_agree_partners_same_strain_call_with_no_shown_length_there`, and the existing `compile::desc::context::tests::splinter_falls_back_to_partners_last_natural_suit_when_no_agreed_suit` was strengthened to give its partner node a genuine `5+!h` (it previously passed only because it had *no* suit-length constraint at all, which the new guard would otherwise have rejected). Removed 8 real-file Errors: `bml-test/data/example3.bml:27`, `gjp/common/2suitedovercalls.bml:106`, `jdh8/blue/1C.bml:83` and `:120`, `jdh8/blue/1M.bml:73`, `jdh8/common/2NT-UNT.bml:21`, `jdh8/wj/1C.bml:308`, `jdh8/wj/1M.bml:111`.
- **NAT explicit-wins (`compile/desc/mod.rs`, `compile/desc/context.rs`)**: `docs/design/06-system.md` §7.5 states NAT's own suit length gives way to an explicit length the row states for the same suit ("衝突は明示が勝つ", the rule the strength-word fix above already generalizes from NAT -- but NAT itself never implemented it). `resolve_natural` always ANDed `suit_len[call's suit] >= natural_suit_length` (5 by default for most roles/levels) even when the row stated its own, shorter length explicitly. Fixed by dropping NAT's atom when the description also carries an explicit `SuitLen`/`Shape` fragment pinning the same suit. Regression test: `compile::desc::tests::explicit_suit_length_wins_over_nats_own_assumed_minimum`. Removed 5 real-file Errors: `gjp/common/1C.bml:54`, `gjp/common/1H-1S.bml:36` and `:46`, `gjp/common/1M-1N.bml:27`, `gjp/common/1m-2m.bml:9` (this last one had also been misclassified as class (b): the file's header prose "promises 5+ clubs" was never the actual contradiction -- the row's own `at least 4!c, NAT` triggered this same NAT bug regardless of the header).
- **`tests/compile_real.rs` / `tests/common/mod.rs` file attribution**: a lint's `span.file` (a `FileId`) names the file it actually came from, but the test built its comparison key from the *root* file's own path paired with `span.line`, so every Error raised inside an `#INCLUDE`d file was attributed to whichever root happened to pull it in, at that root's own line count -- a line number belonging to a different file, sometimes past that file's own length (e.g. `jdh8/blue.bml` is 118 lines; the old file wrongly cited `blue.bml:214`, really `jdh8/blue/1C.bml:214`). The same source Error was also counted once per root that includes it. Fixed by resolving `span.file` through the loader's own file table (`tests/common/mod.rs`'s new `compile_guarded_with_files`) and by a separate `lexer::normalize_path` bug this surfaced: normalizing an absolute `#INCLUDE` target's resolved path silently dropped its leading `/` (the same code path that drops a `.`/empty segment also dropped the leading-slash artifact of an absolute path), corrupting every included file's recorded path whenever the root path was itself absolute (the normal case for `FsLoader`). Regression test: `lexer::tests::included_files_keep_the_root_paths_leading_slash`. This dropped the reported count from "100 Errors across 28 files" (never a real number -- see below) to the true **64 Errors across 21 files**.


## What "100 Errors across 28 files" actually was

Before this round's attribution fix, 24 of this file's 100 entries pointed at a root aggregator (`jdh8/blue.bml`, `jdh8/wj.bml`, `jdh8/defense.bml`) at a line number that actually belonged to one of its `#INCLUDE`d children, and the underlying source Error was additionally double- (sometimes triple-) counted once per root that reaches it. Resolving `span.file` correctly leaves **77 distinct (true file, line, code) locations** pre-fix, of which this round's two compiler fixes above remove 13 (8 from the splinter/agreed-suit fix, 5 from the NAT fix), landing at the **64** listed above.
