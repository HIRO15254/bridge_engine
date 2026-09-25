# Real BML file lint triage (roadmap 3.2-3.4)

Every `Error`-severity lint the compiler produces over `systems/vendor/data` (bml-test, jdh8, gjp), triaged into:

- **(a) our compiler bug** — fixed directly (see `compile/desc/{clause,context,tokens}.rs` and `compile/expand.rs`), each with its own small inline-BML regression test. None remain open; this table lists only what could not be fixed this round.
- **(b) genuine contradiction in the source file itself** — recorded, not fixed (the file is self-contradictory or the vendored fixture is intentionally so).
- **(c) design/vocabulary gap** — a real compiler limitation (grammar, Pass 2 context resolution, or vocabulary coverage) that is known and recorded rather than chased further this round; each is a candidate v2 task.

Every row below is also a line in `tests/data/real_expected_errors.txt`, which `tests/compile_real.rs` asserts the actual Error set equals exactly. Grouped by root cause; each group states the class and the number of `file:line` occurrences, then lists them.


Total: **100** Error-severity lints, 15 distinct root causes, across 28 files.


By class: (b) genuine source contradiction = 2, (c) design/vocabulary gap = 98.


## Root-cause groups


### 1. [c] Same architectural family as the splinter/agreed-trump-suit fix: the splinter/transfer's own suit and the inferred agreed/target suit collide under a context that fix does not cover (partner's call names no suit, or the position is reached via a trie node shared with another table); recorded as a known Pass-2 context-resolution gap.


45 occurrence(s):


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
| `gjp/common/2suitedovercalls.bml:106` | UnsatisfiableConstraint | SPL for !s |
| `jdh8/blue.bml:120` | UnsatisfiableConstraint | Serious SPL, 0--1!c, 3+!h |
| `jdh8/blue.bml:21` | UnsatisfiableConstraint | MIN SPL, 0=!h |
| `jdh8/blue.bml:33` | UnsatisfiableConstraint | F, MIN SPL, 1=!c, 3+!s, 3+!h, 5+!d |
| `jdh8/blue.bml:73` | UnsatisfiableConstraint | SPL, 0--1#, 4+!h |
| `jdh8/blue.bml:83` | UnsatisfiableConstraint | SPL, 0--1!s, 4+!h |
| `jdh8/blue/1C.bml:120` | UnsatisfiableConstraint | Serious SPL, 0--1!c, 3+!h |
| `jdh8/blue/1C.bml:83` | UnsatisfiableConstraint | SPL, 0--1!s, 4+!h |
| `jdh8/blue/1D.bml:33` | UnsatisfiableConstraint | F, MIN SPL, 1=!c, 3+!s, 3+!h, 5+!d |
| `jdh8/blue/1M.bml:73` | UnsatisfiableConstraint | SPL, 0--1#, 4+!h |
| `jdh8/common/2NT-UNT.bml:21` | UnsatisfiableConstraint | MIN SPL, 0=!h |
| `jdh8/wj.bml:10` | UnsatisfiableConstraint | MAX SPL, 0--1!c, 5+!s, 5+!h |
| `jdh8/wj.bml:11` | UnsatisfiableConstraint | MAX SPL, 0--1!d, 5+!s, 5+!h |
| `jdh8/wj.bml:111` | UnsatisfiableConstraint | SPL, 0--1!d |
| `jdh8/wj.bml:21` | UnsatisfiableConstraint | MIN SPL, 0=!h |
| `jdh8/wj.bml:308` | UnsatisfiableConstraint | SPL, 7--10 HCP, 0--1!d, 6+M |
| `jdh8/wj.bml:43` | UnsatisfiableConstraint | SPL, 21--23 HCP, 40(54) |
| `jdh8/wj.bml:44` | UnsatisfiableConstraint | SPL, 21--23 HCP, 04(54) |
| `jdh8/wj.bml:92` | UnsatisfiableConstraint | INV, decline a !d SPL |
| `jdh8/wj.bml:93` | UnsatisfiableConstraint | INV, decline a !h SPL |
| `jdh8/wj/1C.bml:308` | UnsatisfiableConstraint | SPL, 7--10 HCP, 0--1!d, 6+M |
| `jdh8/wj/1C.bml:43` | UnsatisfiableConstraint | SPL, 21--23 HCP, 40(54) |
| `jdh8/wj/1C.bml:44` | UnsatisfiableConstraint | SPL, 21--23 HCP, 04(54) |
| `jdh8/wj/1M.bml:111` | UnsatisfiableConstraint | SPL, 0--1!d |
| `jdh8/wj/1M.bml:92` | UnsatisfiableConstraint | INV, decline a !d SPL |
| `jdh8/wj/1M.bml:93` | UnsatisfiableConstraint | INV, decline a !h SPL |
| `jdh8/wj/2C.bml:11` | UnsatisfiableConstraint | MAX SPL, 0--1!d, 5+!s, 5+!h |


### 2. [c] A parenthetical aside describing an alternative call or exception (else/otherwise/also/maybe/opener can have) is not excluded from the row's constraint and is ANDed in as if it were a literal requirement on this hand.


12 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C.bml:147` | UnsatisfiableConstraint | 5M-5m, good suits (else 2!d), FG |
| `gjp/common/1C.bml:174` | UnsatisfiableConstraint | MAX, S/S (also 3M) |
| `gjp/common/1C.bml:191` | UnsatisfiableConstraint | NAT, INV (1!c-2!h shows 5!s-4!h and a weak hand) |
| `gjp/common/1C.bml:216` | UnsatisfiableConstraint | 5M-4m, good suits (else 2!s), S/T |
| `gjp/common/1C.bml:217` | UnsatisfiableConstraint | 5M-5!d, good suits (else 2!s), S/T |
| `gjp/common/1C.bml:218` | UnsatisfiableConstraint | 6M, good suit (else 2!s), S/T |
| `gjp/common/1C.bml:331` | UnsatisfiableConstraint | nothing to bid (like 1C-1D without intervention) but with at least 3!c, opener rebids l... |
| `gjp/common/1C.bml:54` | UnsatisfiableConstraint | FG, NAT (maybe 3 cards only) |
| `gjp/common/1C.bml:82` | UnsatisfiableConstraint | FG, 5+!h (opener can have 3 cards so with 4!h you bid 4SFG first), longer diamonds |
| `gjp/common/1C.bml:87` | UnsatisfiableConstraint | FG, 5+!s (opener can have 3 cards so with 4!s you bid 4SFG first), longer diamonds |
| `gjp/common/1H-1S.bml:36` | UnsatisfiableConstraint | T/P, NAT, normally 3!h (otherwise 2!d) |
| `gjp/common/1M-1N.bml:27` | UnsatisfiableConstraint | T/P, NAT, normally 3M (otherwise 2!d) |


### 3. [c] Pass-2 context resolution does not yet model this combination of context-dependent words/fragments for this row; recorded as a known gap rather than chased further this round.


9 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C-1M-2M.bml:10` | UnsatisfiableConstraint | MAX, 4M |
| `gjp/common/1C-1M-2M.bml:8` | UnsatisfiableConstraint | MAX, 3M, NAT |
| `gjp/common/1C.bml:247` | UnsatisfiableConstraint | 5422, 4M, FG |
| `gjp/common/1C.bml:333` | UnsatisfiableConstraint | as before but with at most 2!c, opener rebids like after 1!c-1!d without further interv... |
| `gjp/common/1C.bml:38` | UnsatisfiableConstraint | 6!c, 6-9 HCP, expects to win 3NT opposite a strong balanced hand |
| `gjp/common/1H-1S.bml:46` | UnsatisfiableConstraint | NAT, normally 4!s |
| `gjp/common/2N-Puppet.bml:36` | UnsatisfiableConstraint | no 4M, no interest in playing 4!s opposite 5!s-4!h |
| `jdh8/wj.bml:120` | UnsatisfiableConstraint | QUANT INV to 6NT |
| `jdh8/wj/1C.bml:120` | UnsatisfiableConstraint | QUANT INV to 6NT |


### 4. [c] Two context-dependent strength words joined by a comma are intended by the author as alternative branches (if spades, invitational; if hearts, forcing / usually X but sometimes Y), but the grammar's comma is a strict AND, so their HCP ranges are incompatibly conjoined.


8 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C-1M-2M.bml:4` | UnsatisfiableConstraint | MIN, usually 4M but sometimes 3M is possible |
| `jdh8/blue.bml:66` | UnsatisfiableConstraint | P/C, INV in !s, FG in !h |
| `jdh8/common/2X-Multi-Muiderberg.bml:66` | UnsatisfiableConstraint | P/C, INV in !s, FG in !h |
| `jdh8/defense.bml:43` | UnsatisfiableConstraint | MIN INV, usually 4=# |
| `jdh8/defense/2X-NAT.bml:43` | UnsatisfiableConstraint | MIN INV, usually 4=# |
| `jdh8/wj.bml:243` | UnsatisfiableConstraint | PRE 7+!c, INV+ 4+!d, or UNBAL FG |
| `jdh8/wj.bml:66` | UnsatisfiableConstraint | P/C, INV in !s, FG in !h |
| `jdh8/wj/1D.bml:243` | UnsatisfiableConstraint | PRE 7+!c, INV+ 4+!d, or UNBAL FG |


### 5. [c] Gambling 3NT cue-bid convention: the call's own suit and the opponents' cue suit resolve to the same bound history variable, so solid 7+ suit (needs AKQ) and without stopper (needs no AKQ) directly contradict in the same suit.


8 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `jdh8/blue.bml:214` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/blue/1C.bml:214` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/defense.bml:11` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/defense/1X.bml:11` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/wj.bml:199` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/wj.bml:440` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/wj/1C.bml:440` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |
| `jdh8/wj/1M.bml:199` | UnsatisfiableConstraint | Gambling, SOL 7+ suit without stopper |


### 6. [c] The (3)4+ optional-count shorthand (3-or-4-plus cards) is not recognized by the shape/length grammar and is parsed in a way that contradicts the row's own explicit suit-count fragments.


4 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2suitedovercalls.bml:53` | UnsatisfiableConstraint | NF, (3)4+!h, light INV |
| `gjp/common/2suitedovercalls.bml:54` | UnsatisfiableConstraint | NF, (3)4+!s, light INV |
| `gjp/common/2suitedovercalls.bml:80` | UnsatisfiableConstraint | NF, (3)4+!h, light INV |
| `gjp/common/2suitedovercalls.bml:81` | UnsatisfiableConstraint | NF, (3)4+!s, light INV |


### 7. [c] A weak-two augmentation word (MAX / fit) combines with the base weak-two length/HCP range in a way Pass 2 does not model, producing a range that excludes the row's own explicit suit fragment.


3 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2C.bml:59` | UnsatisfiableConstraint | weak-two in !d, fit, MAX |
| `gjp/common/2D.bml:56` | UnsatisfiableConstraint | weak-two in !s, MAX, good suit |
| `gjp/common/2D.bml:57` | UnsatisfiableConstraint | weak-two in !h, MAX, good suit |


### 8. [c] A multi-line (a)/(b)/(c) lettered list of alternative meanings is joined by the grammar as a strict AND across lines instead of as OR across lettered branches.


3 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `jdh8/blue.bml:13` | UnsatisfiableConstraint | F, Multi:\n(a) NAT FG, 4+!c, 0--3!s, 0--3!h\n(b) PRE TRF, 0--7 HCP, 4--5!d\n(c) PRE, 0-... |
| `jdh8/blue/1D.bml:13` | UnsatisfiableConstraint | F, Multi:\n(a) NAT FG, 4+!c, 0--3!s, 0--3!h\n(b) PRE TRF, 0--7 HCP, 4--5!d\n(c) PRE, 0-... |
| `jdh8/wj.bml:74` | UnsatisfiableConstraint | F, Polish Club:\n(a) 12--14 HCP, 2--4!s, 2--4!h, 2--4!d, 2--4!c\n(b) 11--17 HCP, 5+!c o... |


### 9. [c] "same meaning and development as after X" is a cross-reference aside, a different phrasing than "see X" (handled by the description compiler's see-reference fix) that the vocabulary does not yet recognize as opaque.


2 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2C.bml:70` | UnsatisfiableConstraint | same meaning and development as after 2!c-2!d-3X |
| `gjp/common/2D.bml:59` | UnsatisfiableConstraint | same meaning and development as after 2!d-2!h-3X |


### 10. [b] Synthetic bml-test oracle fixture: the splinter call's own suit is literally the same suit partner just bid, a same-suit contradiction baked into the official test file itself.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `bml-test/data/example3.bml:27` | UnsatisfiableConstraint | Splinter |


### 11. [c] "<shape> possible after <auction>" is a conditional footnote about a different call sequence, not a literal shape constraint on this row; the vocabulary does not yet recognize this phrasing as opaque (same family as the see-reference handling).


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1C.bml:115` | UnsatisfiableConstraint | 12-14 NT. 4333 possible after 1!c-1!h. |


### 12. [c] Deeply nested (R) redouble-relay under a doubled 2C rebid; call-legality resolution for a repeated (R) step under this history is a known gap, not yet modeled.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1N.bml:163` | IllegalCall | Stayman again, INV+ |


### 13. [c] The file's free-text intro paragraph before the first auction table is parsed as a row with an empty description; an empty description should compile to Atom::ANY (trivially satisfiable) but this row ends up Unsatisfiable instead -- recorded as a parser-classification gap rather than fixed this round.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1m-2m.bml:3` | UnsatisfiableConstraint |  |


### 14. [b] The file's own header prose promises 5+ clubs after a 1C opening, but this row's explicit fragment says only 4+ clubs -- a genuine inconsistency within the source file itself.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/1m-2m.bml:9` | UnsatisfiableConstraint | at least 4!c, NAT |


### 15. [c] The qualifier before the strength word (light before INV) is not recognized by the vocabulary, so the plain INV range is used as if unqualified, conflicting with the row's own explicit suit-length fragment.


1 occurrence(s):


| file:line | lint | row description |
| --- | --- | --- |
| `gjp/common/2suitedovercalls.bml:102` | UnsatisfiableConstraint | light INV, !s |

