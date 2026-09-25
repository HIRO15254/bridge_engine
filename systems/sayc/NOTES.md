# Notes on this SAYC file

The ACBL SAYC System Booklet (revised January 2006) is a high-level, prose
document; it is not a row-by-row bidding table, and it leaves real room for
partnership judgment in several places. This file records the choices this
BML implementation makes there, and the places where the v1 description
vocabulary (`docs/design/06-system.md` §7.4) cannot express what the booklet
says exactly. Numbers below match the `NOTES.md #N` references left as
comments in the `.bml` files.

1. **Longer-minor comparisons (`openings.bml`).** The booklet's rule for
   opening 1!c vs. 1!d only gives two named cases: 3-3 minors open 1!c, 4-4
   minors (and the 4-4-3-2 shape) open 1!d. The general "open the longer
   suit" principle also applies when the minors are *unevenly* long (for
   example 4 diamonds and 6 clubs should open 1!c). The v1 vocabulary has no
   suit-length comparison token (§7.4 lists "longer suit" comparisons under
   "v2, unrecognized"), so this file cannot express that comparison and
   instead gives 1!d a flat higher `{prio:N}` than 1!c. This gets the two
   named cases and the common "longer diamonds" case right, but a hand with
   clubs strictly longer than diamonds while diamonds is still 4+ (e.g.
   4=!d/6=!c) will bid 1!d instead of the technically-preferred 1!c. The
   same gap applies, in principle, to two suits of unequal length that are
   *both* eligible to open (e.g. a 5-card major and a longer minor, where
   standard practice opens the longer minor first): this file always prefers
   the 5+ card major, which matches the booklet's own "normally five-card
   majors" framing but not the finer-grained exception some partnerships
   play. A real suit-length-comparison token in the description vocabulary
   would let a future revision fix both.

2. **1NT/2NT/2!c priority (`notrump.bml`).** A 25-27 balanced hand also
   satisfies 2!c's "22+ hcp" template, and a 20-21 balanced hand with a
   5-card suit also satisfies the matching suit opening. SAYC's own text
   ("notrump openings ... may be made with a five-card major suit") makes
   clear the direct notrump bid is preferred, so 3NT/2NT/1NT are all given
   higher `{prio:N}` than 2!c and the suit openings.

3. **Weak-two length (`weak-twos.bml`).** The booklet allows "a very good
   five-card suit" or "a poor seven-card suit" on rare occasions. Modeling
   suit *quality* well enough to gate those exceptions is out of scope for
   v1 (no negative/"poor" quality token exists), so this file requires
   exactly six cards, leaving the 5-/7-card exceptions as booklet-acknowledged
   judgment calls that a compiler-driven system cannot currently make. This
   also keeps weak twos and three-level preempts (which need exactly seven)
   cleanly disjoint.

4. **"Feature" bids (`weak-twos.bml`).** After 2NT asking, a maximum opener
   without a feature bids 3NT, and with a feature bids the suit containing
   the ace or king. The vocabulary has no "specific ace or king in this suit"
   token (`Stopper`/`Quality` are about NT stoppers and suit quality, not a
   single high card), so the feature suits are modeled only by their HCP
   range (9--11), without the ace-or-king requirement itself.

5. **Preempt length/HCP bands (`preempts.bml`).** The booklet gives no table
   for opening preempts, only the "rule of 2/3/4" trick-counting judgment
   and vulnerability. The lengths and ranges here (exactly 7 for a 3-level
   preempt, 8+ for a 4-level one, 5--9/5--10 HCP) are an interpolation, kept
   uniform across all four suits for simplicity; some sources give the
   4!h/4!s opening a 7-card minimum instead of 8 (a slightly looser preempt
   since game is bid directly), which this file does not model separately.
   Vulnerability itself is not part of the description vocabulary (`#SEAT`
   and `#VUL` gate whole rows, not a continuous trick count), so the
   "sound at unfavorable / light at favorable" discipline from the booklet
   is left to the sampler or the user rather than encoded here.

6. **Minor-suit jump shifts (`responses-minor.bml`).** The booklet states
   jump shifts explicitly only for major-suit openings ("2S, 3C, 3D = strong
   jump shifts" after 1!h) and says minor-suit responses "generally follow
   the ideas set down in the previous section" without restating them. This
   file does not add jump-shift rows over 1!c/1!d; by symmetry they would
   need the same "not 3+ support" exclusion used for the major-suit ones
   (see note 7) and are a reasonable, but unstated, extension left out here.

7. **Priority as a stand-in for "which call would a player actually make."**
   Several response tables have two calls whose *constraints* both fit a
   hand (for example a hand with both four spades and four-card heart
   support after 1!h, which satisfies both "1S" and "2NT" (Jacoby)). SAYC's
   own practice is to show the fit first with sufficient values, so this
   file gives the fit-showing/most-descriptive call a higher `{prio:N}` and,
   where practical, also excludes the fit directly from the competing row
   (`not 3+!h` on major-suit jump shifts) so the two mechanisms agree.

8. **Balancing seat.** The booklet says a reopening bid "means much the same
   as a direct-seat bid, though it can be lighter at the minimum end," and
   gives one concrete number (10--15 for a reopening 1NT). It gives no
   number for a lighter reopening takeout double or suit overcall; the
   numbers used here (8+ hcp for the double, 6+ hcp for a suit overcall, vs.
   12+/8+ direct) are this file's own interpolation of "somewhat lighter."

9. **Negative doubles beyond the three worked examples.** The booklet gives
   1!c-(1!d)-X, 1!d-(1!h)-X and 1!d-(1!s)-X explicitly. `competition.bml`
   adds 1!c-(1!h)-X and 1!c-(1!s)-X by the same reasoning (double instead of
   bidding the cheaper available major directly), which is a standard
   extension but not literal booklet text.

10. **Unusual 2NT is written without an HCP bound.** The booklet describes
    it purely by shape (5+5+ in the two lowest unbid suits); some
    partnerships play it as a two-way bid (weak *or* strong) and use
    further bidding to sort out which, but the booklet doesn't say, so this
    file leaves the HCP axis unconstrained rather than inventing a number.

11. **Rule of 20 is intentionally absent.** The booklet's own opening
    framework has no rule-of-20 (or similar shape-based) allowance for
    opening light; per the task brief ("rule of 20 only if SAYC says so"),
    none is added and the 12-hcp floor from `#+STRENGTH: opening=12`
    applies uniformly.

12. **Splinter bids are omitted.** The ACBL SAYC booklet's competitive and
    response sections never mention splinters (unlike some secondary SAYC
    summaries); this file does not use the `SPL` token anywhere, even though
    `#+CONVENTION: splinter=4` still has a compiler-side default value.

13. **The 2S minor-suit relay's exact shape (`notrump.bml`).** The booklet states
    only that opener must rebid 3C after a 2S response, which is passed with "a
    club bust" or corrected to 3D with "a diamond bust" -- it never gives 2S's
    own length/HCP requirement, unlike the other 5-5 two-suited conventions it
    documents explicitly (a jump to 2NT shows "at least 5-5 in the lowest two
    unbid suits"; a Michaels cuebid shows "a 5-5 two-suiter (or more
    distributional)"). This file adopts the same "5-5" reading by analogy
    (`5+5+ minors`) rather than leaving 2S unconstrained. The pass/correct step
    itself is written as plain suit-length facts (`5+!c` / `5+!d`) instead of the
    booklet's prose ("club bust" / "diamond bust"), which have no vocabulary
    tokens and would otherwise compile to unrecognized freetext with no
    constraint at all.

14. **Cuebidding one of the opponents' suits after they use Michaels or the
    unusual notrump against our own opening (`competition.bml`).** The
    booklet's own worked example (`1S — (2S) — 3H = game force`) cuebids a
    *specific* suit that depends on what the opponents' convention showed, so
    it cannot be written with a shared pattern variable the way the direct
    overcall in section "Overcalls" is (there, `Y` is genuinely any suit; here
    the suit is determined by which suit the opponents used, which in turn
    depends on which suit *we* opened). This file spells out each opening
    suit's case by hand: after a major opening the Michaels cuebid shows "the
    other major and an unspecified minor," so cuebidding "one of their known
    suits" means the other major specifically (`1H-(2H)- 3S = F`,
    `1S-(2S)- 3H = F`); after a minor opening it shows both majors, so either
    major cuebid qualifies (`1C-(2C)-` / `1D-(2D)-`, both `3H = F` and
    `3S = F`). The parallel case against the unusual notrump (`1X-(2N)-`) is
    left with only the generic double, since the two suits the unusual
    notrump shows are the two lowest *unbid* suits and differ for all four
    opening suits; writing out that case would need four more literal pairs
    for a very rare auction.

15. **Gerber is written as `!BW` (`notrump.bml`, `strong-2c.bml`).** The v1
    convention vocabulary (`docs/design/06-system.md` §7.4) lists `BW`,
    `RKCB`, `KCB`/`K/B` as recognized ace-asking conventions but has no
    separate token for Gerber (the 4!c ace-ask used directly over notrump,
    as opposed to Blackwood's 4NT after a suit is agreed). Every `4C = !BW`
    row that follows directly from a notrump bid (1NT, 2NT/3NT, and the
    2!c-2!d-2NT sequence) is really Gerber; this file reuses the `BW` token
    rather than leaving the alert unrecognized, since both are simple
    ace-asking conventions and the vocabulary gap does not change any actual
    constraint (`Convention` tokens carry no `Atom` on their own).

16. **Depth of the auction tree.** This file models opening bids, one level
    of responses, a representative (not exhaustive) slice of opener's
    rebids and responder's follow-ups, and the competitive-bidding
    conventions the roadmap names. Real partnerships go many calls deeper in
    specific auctions (e.g. after Jacoby transfers, or Stayman with
    interference); those deeper sequences are left for a future revision and
    are exactly the kind of gaps `xtask coverage` (phase 3.10) is meant to
    surface as `Fallback`/`NoCandidate` nodes rather than wrong bids.

17. **`#INCLUDE` needs a blank line on both sides (`sayc.bml`,
    `openings-only.bml`).** `#INCLUDE` is a literal textual splice (no
    inserted blank lines, matching `bss.py`'s own behaviour); with two
    `#INCLUDE` lines back to back, the last physical line of one included
    file and the first physical line of the next end up in the very same
    paragraph, and the next file's own `* Heading` line is then parsed as if
    it were one more row of the previous file's last bidding table (an
    unparsable `*` call token, silently dropped along with its subtree, and
    a phantom `Row` with no real content). Every `#INCLUDE` here is followed
    by a blank line for exactly this reason; a future new include must keep
    that blank line.

18. **Responder's follow-up after a Jacoby-transfer acceptance is written
    with plain numbers, not `INV` (`notrump.bml`).** After 1NT-2!d(transfer
    to hearts)-2!h(accept) (and the mirrored spade sequence), responder's
    `Pass`/`2N` choice is `0--7 hcp` / `8--9 hcp`, not `INV`: the `INV`
    context word looks at *partner's own most recent node* (`partner_last`)
    to find the opener's shown range, and opener's acceptance bid is written
    as `unlimited` (correctly -- accepting a transfer restates nothing about
    the 1NT opening's own 15--17), so `INV` would resolve against an
    unconstrained partner range instead of the 1NT opening's, giving an
    empty (`UnsatisfiableConstraint`) result. `3C = INV, 6+!c` and
    `3D = INV, 6+!d` two rows down are fine as `INV` -- there responder is
    replying directly to the `1N` opening itself, with no intervening
    unlimited rebid in between. This file follows the task brief's own
    "prefer explicit numbers over context words" guidance here rather than
    trying to make `INV` context-aware of a whole chain of ancestors.

19. **The hearts-transfer continuation table's `4S = 6+!s` was `4H = 6+!h`
    misspelled, and the mirrored spades-transfer table was missing its own
    equivalent line (`notrump.bml`).** After accepting a transfer, a
    responder rebid of four of the *agreed* major with a good 6-card suit is
    a natural, minimum-strength "I just want you to know I have six" call;
    written as `4S` under the *hearts* transfer table it named the wrong
    major entirely (and was unreachable in practice, since the file has no
    other `4S` there to duplicate it against). Fixed to `4H` under hearts
    and added as `4S` under the spades table, so both transfers now have the
    same shape of continuation.
