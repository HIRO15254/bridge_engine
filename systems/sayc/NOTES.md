# Notes on this SAYC file

The ACBL SAYC System Booklet (revised January 2006) is a high-level, prose
document; it is not a row-by-row bidding table, and it leaves real room for
partnership judgment in several places. This file records the choices this
BML implementation makes there, and the places where the v1 description
vocabulary (`docs/design/06-system.md` §7.4) cannot express what the booklet
says exactly. Numbers below match the `NOTES.md #N` references left as
comments in the `.bml` files.

1. **Longer-minor and longer-major comparisons (`openings.bml`).** The v1
   vocabulary has no suit-length *comparison* token (§7.4 lists "longer suit"
   comparisons under "v2, unrecognized"), so "open the longer suit" is
   written out as the exhaustive set of length pairs for which it is true
   (`N!x 0--(N-1)!y` for each length `N`, plus the tie cases), rather than as
   a flat `{prio:N}` race between the two calls. This now gets both the
   major-vs-major choice (open the longer of two 5+ card majors, higher-
   ranking on a 5-5 tie) and the minor-vs-minor choice (open the longer
   minor; 3-3 ties to 1!c, 4-4-or-longer ties to 1!d) exactly right for every
   shape, including uneven lengths the previous priority-only encoding got
   wrong (5!c/4!d used to bid 1!d; 6!h/5!s used to bid 1!s). The `4432`
   fragment on 1!d's row is a *positional* pattern (spades-hearts-diamonds-
   clubs order), matching only 3+!d/2=!c specifically: it extends "longer
   diamonds" one length pair further down (a 3-2 minor split, below 1!d's own
   4+ length branches) rather than being a general "4432 shape, whichever
   minor is the doubleton" override -- the mirror shape 4=4=2=3 (3+!c/2=!d)
   is a longer-*clubs* hand and opens 1!c through the ordinary length
   branches, like any other longer-clubs shape. `crates/bridge-system/tests/
   sayc.rs`'s `sayc_opening_choice_by_suit_length` is a table-driven
   regression test over these shapes (6-5/5-6/5-5 majors, 4-4/3-3 minor
   ties, 4=4=3=2, 4=4=2=3, and uneven non-tie minors down to 6-1).

   One gap remains, unchanged from before: two suits of unequal length that
   are *both* eligible to open (e.g. a 5-card major and a longer minor,
   where standard practice opens the longer minor first) -- this file always
   prefers the 5+ card major over any minor, regardless of the minor's
   length, which matches the booklet's own "normally five-card majors"
   framing but not the finer-grained exception some partnerships play. That
   comparison is *across* the major/minor priority tiers rather than within
   one, and closing it would need the same kind of length-pair enumeration
   again, this time crossed with every major length; left for a future
   revision since the task brief scoped this fix to the within-major and
   within-minor choices.

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
   range (9--11), without the ace-or-king requirement itself. A direct,
   spelled-out consequence (`dropped.json` #16, raised again in review): since
   the three feature-suit rows and the no-feature `3N` row all compile to the
   exact same `9--11 hcp` constraint, only one of the four -- whichever has
   the highest `{prio:N}` -- can ever be chosen by `choose_bid`; the other
   three are permanently unreachable, not merely low-frequency. This is the
   honest consequence of the missing vocabulary, not a bug this file can fix
   without inventing an unfounded distinguishing constraint (which would be
   worse: a fabricated feature the opener's actual hand may not have); the
   `{prio:N}` ordering is kept only so the compiled system deterministically
   picks *a* feature-tier response rather than leaving a 9--11 hcp maximum
   with `NoCandidate`.

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

13. **The 2S minor-suit relay's exact shape (`notrump.bml`).** The booklet's own
    text for 2S is a weak, one-suited sign-off ("a weak hand with long clubs or
    long diamonds"): responder passes 3C with clubs (a "club bust") or corrects
    to 3D with diamonds (a "diamond bust"), not a strong two-suited hand. This
    file originally read 2S by analogy with the booklet's other 5-5 two-suited
    conventions (a jump to 2NT's "at least 5-5 in the lowest two unbid suits", a
    Michaels cuebid's "5-5 two-suiter"), requiring `5+5+ minors`; that reading is
    wrong for 2S specifically, since with both minors required the earlier `P =
    5+!c` row was satisfied by every hand that could bid 2S at all (any 5+5+
    minor hand has 5+ clubs), making the `3D = 5+!d` sign-off unreachable and
    leaving diamond-only weak hands with no call. This file now writes 2S as
    `0--7 hcp, 6+!c or 6+!d` (weak, one long minor, either suit) with `P = 6+!c`
    and `3D = {prio:1} 6+!d` so a diamond-only hand corrects. The pass/correct
    step itself is written as plain suit-length facts (`6+!c` / `6+!d`) instead
    of the booklet's prose ("club bust" / "diamond bust"), which have no
    vocabulary tokens and would otherwise compile to unrecognized freetext with
    no constraint at all.

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

17. **`#INCLUDE` always starts and ends a paragraph (`sayc.bml`,
    `openings-only.bml`).** Like `bml.py` (which substitutes
    `'\n' + text + '\n'` for the directive), the lexer frames every
    included file with a paragraph break on both sides, so two back-to-back
    `#INCLUDE` lines keep the two files' paragraphs apart. (An earlier
    version of the lexer spliced the text with no break, merging the last
    table of one file with the next file's `* Heading`; the blank lines
    after each `#INCLUDE` here date from that workaround and are now merely
    cosmetic.)

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
    a natural game sign-off ("I have six and enough values for game, no need
    to explore further") -- not a minimum-strength call, since responder
    could equally hold a hand strong enough to have driven to slam and chose
    the simple route; `3N = CoG` is the companion call for the same range
    (10--15 hcp) with only a 5-card suit, where opener's choice of game
    still matters. Written as `4S` under the *hearts* transfer table it
    named the wrong major entirely (and was unreachable in practice, since
    the file has no other `4S` there to duplicate it against). Fixed to
    `4H` under hearts and added as `4S` under the spades table, both now
    `10--15 hcp, 6+!x`, so both transfers have the same shape of
    continuation and `3N`/`4M` are cleanly split by suit length (5 vs 6+)
    rather than overlapping.

20. **Advancing a takeout double had no suit-length constraint at all
    (`competition.bml`, review finding).** The minimum and invitational
    new-suit advance rows (`(1C)-D-` etc.) originally read `1D = {prio:6}
    unlimited`, `2D = {prio:3} INV`, and so on, with no `#!x` atom -- every
    row for every unbid suit compiled to the same unconstrained shape, so a
    hand with (say) 4 hearts and no diamonds still "qualified" for `1D`, and
    since `{prio:N}` alone (not a real constraint difference) decided which
    row won, the cheapest suit's row always won regardless of what the hand
    actually held: advancer was always shown bidding the cheapest unbid suit,
    never the suit(s) it actually had. Worse, because the invitational jump
    rows were also unconstrained, and ranked *below* the plain minimum rows in
    `{prio:N}`, they could never be reached at all: any hand meeting a jump's
    invitational criteria also trivially satisfied the (higher-priority,
    unconstrained) plain minimum row in some suit. Fixed by adding the real
    length atom to every row (4+ for a minimum call, 5+ for an invitational
    jump) and by re-ranking the invitational tier above the minimum tier (the
    same "more descriptive call wins" convention as note 7), so a hand's
    actual shape decides which suit is shown, and an invitational-quality
    5+ card suit is shown by jumping rather than folded into a same-suit or
    different-suit minimum call. `(1S)-D-` (advancing a double of a 1!s
    opening) was missing outright -- every unbid suit there ranks below
    spades, so its minimum calls are already at the 2 level -- and is added
    alongside the equivalent fix for `(1D)-D-`/`(1H)-D-`'s own missing
    below-rank suits (clubs after a 1!d double; clubs and diamonds after a
    1!h double).

21. **Interference over 1NT was keyed on an impossible history
    (`notrump.bml`, `confirmed.json` #8).** `1N-(1X)-` asks for a 1-level
    overcall of a 1NT opening, but notrump is the top strain at the 1 level,
    so no such call exists; the whole table (a natural `2Y` and a
    game-forcing cuebid) silently expanded to nothing, leaving `1NT-(2X)-?`
    entirely off-system. Rewritten as `1N-(2X)-` (the overcaller's suit at
    the 2 level, the cheapest it can actually be), with the cuebid moved up a
    level to `3X` to stay above the overcall, and, mirroring the direct-seat
    overcall table's own "a lower suit costs an extra level" handling
    (`competition.bml`), explicit `3C`/`3D`/`3H` rows added per overcall suit
    for the natural suits ranked *below* it that a shared `Y` binding (which
    only reaches suits ranked above `X`) cannot reach.

22. **Stayman after interference over 1NT did not require a four-card major
    (`notrump.bml`, `competition.bml`; `dropped.json` #13).** The direct
    `1N- 2C` Stayman row already requires `4+!h or 4+!s`, but its two
    siblings -- `1N-(D)- 2C` (Stayman still on after an opposing
    double) and the 1NT-overcall's own `(1X)-1N- 2C` -- were left as
    `!STAY NF, 8+ hcp` with no major-suit requirement at all, so any 8+ hcp
    hand without a major, including balanced ones with no interest in either
    major, still asked for one. Both now also read `4+!h or 4+!s`.

23. **Balancing-seat overcalls had two independent problems (`competition.bml`;
    `dropped.json` #15).** (a) The double and the plain suit overcall were
    both at the same (default) priority, so on a tie the double -- which has
    no shape exclusion at all -- won by row order over a hand that actually
    held a real 4-card suit and should have overcalled it instead; the suit
    overcall (`1Y`) is now given `{prio:1}` above the double's `{prio:-1}`. (b)
    The jump overcall (`2Y`) shared `1Y`'s own full 6--16 hcp range with only
    the suit length differing, so *any* 5+ card hand jumped, strong ones
    included, rather than calmly overcalling at the one level; `2Y` now reads
    a distinct, lower `6--10 hcp` band. That alone did not make it a
    preemptive jump: with `5+#` every 6--10 hand with a five-card suit still
    jumped, so the one-level balancing overcall with a five-card suit only
    happened at 11+. The phase-3 recheck made it `6+#`, the same six-card
    suit as the direct seat's weak jump (C8).

24. **Weak-two responses wrote every new suit at the 3 level, even the ones
    ranked above the opening (`weak-twos.bml`; `dropped.json` #16).** After
    2D, hearts and spades both rank above diamonds and are reachable at the
    cheap 2 level (`2H`/`2S`); after 2H, spades alone ranks above it (`2S`).
    The file instead wrote `3H`/`3S` (after 2D) and `3S` (after 2H) -- an
    unwarranted extra-level jump with no distinct meaning of its own, and
    with no route left at all for the plain, cheap new suit RONF describes.
    Moved down to the correct level (`3C` after 2D, and `3C`/`3D` after 2H,
    stay at the 3 level as before, since clubs and diamonds really do rank
    below those openings). The same rows also had no HCP floor at all (`F,
    5+!x` alone), so even a hopeless hand with a random 5-card suit
    "qualified"; all now read `10+ hcp` (a new suit facing a preempt is
    looking for game or better against partner's capped 5--11 range).

25. **`1C-(1D)-`'s natural major responses required 5+ cards, leaving a plain
    4-card major with no call at all (`competition.bml`; `dropped.json`
    #17).** With *both* majors unbid, the negative double (which needs 4+ in
    *both*, `4+!s, 4+!h`) only covers a hand with both majors 4-4; a hand with
    exactly one 4-card major (not the other) needs to bid that major
    directly, exactly as this section's own doc comment already says ("a
    same-level new suit ... is a simple, non-forcing natural bid instead,
    exactly as it would be with no interference"). The `1H`/`1S` rows instead
    required `5+!h`/`5+!s`, so a hand with exactly 4 hearts (or 4 spades) and
    no fit for a double had no call whatsoever. Loosened to `4+!h`/`4+!s`,
    matching the uninterfered response table (`responses-minor.bml`). The
    single-unbid-major tables (`1C-(1H)-`, `1C-(1S)-`, `1D-(1H)-`,
    `1D-(1S)-`) were not touched: there the double's own shape is already
    exactly the complementary case (`4=!s` in the double, `5+!s` in the
    direct bid, so a 4-card holding always has a call and a 5+ one always has
    the other), so no equivalent gap exists.

## Notrump lane

Notes for `notrump.bml`, `strong-2c.bml`, `weak-twos.bml` and `preempts.bml`
written while closing the replay-based consistency harness's coverage holes
(`tests/consistency.rs`, `target/coverage_report.json`). The `.bml` files
refer to them as `NOTES.md "Notrump lane" #Nn`.

N1. **Sign-off leaves list the closing Pass explicitly (`notrump.bml`,
    `strong-2c.bml`, `preempts.bml`).** A trie leaf with no rows (1NT-3NT,
    1NT-2C-2H-4H, a transfer followed by game, ...) is off-system for the
    next player, so `choose_bid` falls back to natural inference, which
    leaves strong or shapely openers with `NoCandidate` and makes the
    harness force a `Pass` whose reading nobody defined. Each sign-off now
    carries `P = unlimited` (or the minimum/maximum split `P = 15 hcp` /
    `3N = 16--17 hcp` after an invitation), so the end of the auction is a
    system position. Invitations are answered with plain HCP numbers taken
    from the opening's own range (15 / 16--17 over 1NT, 20 / 21 over 2NT,
    22 / 23--24 over 2C-2D-2NT) rather than `MIN`/`MAX`, for the same reason
    as #18.

N2. **Responder's ranges opposite 1NT, 2NT and 2C-2D-2NT (`notrump.bml`,
    `strong-2c.bml`).** `4C = !BW` (Gerber, #15) had no constraint at all and
    was the only row without an HCP or shape floor, so every hand that fit no
    other response bid Gerber: a 5 hcp balanced hand answered 1NT, 2NT and
    2C-2D-2NT with 4C. The notrump raises are now a complete ladder by
    strength: over 1NT, 2NT 8--9, 3NT 10--15, 4NT 16--17 (quantitative), 6NT
    18--19, and 4C only with 20+; over 2NT (20--21), 3NT 4--10, 4NT 11--12,
    6NT 13+, with no Gerber row (no range is left for it); over 2C-2D-2NT
    (22--24), 3NT 3--8, 4NT 9--10, 6NT 11+, with no Gerber row. Stayman
    over 2NT needs 4+ hcp (3+ over 22--24), since a weaker hand passes.
    `4N = 16--17` over 1NT no longer has `{prio:1}`: it used to outrank
    Stayman, so a 16--17 hand with a four-card major never looked for the
    fit. The Stayman and transfer continuations follow the same ladder
    (invite / game / quantitative 4NT or 5M / slam), with a four-card fit
    after Stayman shown by raising and a 6+ card suit after a transfer by
    3M (invitational), 4M, 5M (slam invitation) or 6M. With a fit and slam
    values after Stayman, responder uses 4C (Gerber) instead of 4NT.

N3. **Interference tables and lenient matching (`notrump.bml`,
    `strong-2c.bml`, `weak-twos.bml`, `preempts.bml`).** When no table
    covers an opponent's call, `choose_bid` retries with that call replaced
    by `Pass` (`resolve_lenient`, "system on") and offers the *uncontested*
    table's rows. Over a 3-level overcall almost all of them are illegal and
    the rest do not fit, so responder had no call at all; the old weak-two
    and preempt catch-alls (N5) had been hiding exactly this. Each opening in
    this lane now has an interference table: over 1NT the 2-, 3- and 4-level
    overcalls, a double (systems on, redouble 8+), Stayman and transfer
    interrupted by an overcall (opener shows a fit or passes), and an
    overcall after a completed transfer (double with values); over 2NT a
    double (systems on) and overcalls; over 2C and 2C-2D; over each weak two
    and preempt. Overcalls not listed individually use the `(bid)` class
    token (any bid), whose table holds only a double: a double is legal
    after any bid, so the table never offers an illegal call. After
    1NT-(2S)-3S (the cuebid substitute for Stayman), a heart suit can no
    longer be shown at the 3 level; opener bids 3NT and leaves a 4-4 heart
    fit to responder.

N4. **Strong 2C continuations (`strong-2c.bml`).** After a positive
    response the auction is forcing to game, so opener's catch-all
    notrump rebid (2NT, or 3NT after a 3-level positive) is a legitimate
    unconstrained row: opener must bid. After 2C-2D-2H (or 2S), responder
    may not pass a forcing rebid, so the previous `2N = 8+ hcp, bal` became
    the waiting 2NT (unconstrained, last in row order) behind a raise and a
    five-card suit with 8+ hcp; opener then rebids a sixth card, a
    four-card side suit, or 3NT. After 2C-2D-2H-2S opener raises with three
    spades, bids 2NT when balanced, and otherwise makes the non-forcing 3H
    rebid (the booklet's "forcing to 3 of opener's major").

N5. **Weak-two responses have real ranges (`weak-twos.bml`;
    `dropped.json` #10/#16, harness review #3).** `4D/4H/4S = unlimited`
    and `3N = unlimited` matched every hand, so responder never passed a
    weak two and the 2NT ask was unreachable. The rows are now: a raise to
    game over a major with four-card support, 5--14 hcp (the further
    preempt); a three-card raise to the 3 level, 6--14 hcp; the 2NT ask,
    15+ hcp with at least a doubleton; a new suit (RONF), 10+ hcp and five
    cards; 3NT, 15+ hcp (shortness in opener's suit, since a doubleton
    asks first). A hand that fits none of them passes by the implicit
    pass. After a new suit opener raises with three-card support, bids 2NT
    (3NT at the 3 level) with 9--11 hcp, and otherwise rebids his suit.
    Over a takeout double responder raises with three-card support or
    redoubles with 15+; over an overcall he raises when the 3-level raise
    is still available or doubles with 13+.

N6. **Preempt responses have real ranges (`preempts.bml`).** The
    `unlimited` raises outranked 3NT and 4NT, and 4NT over 4H/4S had no
    strength floor, so a 1 hcp hand bid Blackwood. Now: Blackwood 18+ hcp
    with a doubleton in opener's suit; a new suit 13+ hcp and five cards;
    3NT 14+ hcp; a raise of a 3-level minor to 4 with 3+ cards and 6--13
    hcp; a raise of a 3-level major to game with 2+ cards and 12--17 hcp; a
    raise of a 4-level minor to 5 with 2+ cards and 13--17 hcp. Everything
    else passes. After a new suit opener raises with three-card support and
    otherwise rebids his suit.

N7. **Feature rebids after the weak-two 2NT ask are now reachable
    (`weak-twos.bml`; supersedes the "permanently unreachable" part of
    #4).** Each feature row carries `stopper in !x` for its own suit, the
    nearest vocabulary item to the booklet's "ace or king": the stopper
    atom accepts A, Kx, Qxx or Jxxx, so a Qxx or Jxxx holding also counts
    as a feature here (a documented over-approximation; the vocabulary has
    no single-honour token, and an `AK`-style honour run needs two
    honours). The priorities (clubs, then the next suits up) pick the
    cheapest feature when there are several; 3NT (maximum, no feature) is
    the remaining case. The earlier spelling `!c stopper` compiled to ANY:
    the tokenizer reads a suit only *after* the word (`stopper in !c`), so
    `!c stopper` was a stopper in the opponents' suit, which does not exist
    in an uncontested auction.

N8. **Pattern rows that share a table with exact rows carry `{prio:1}`
    (`notrump.bml` `1N-(3X)- 3Y`, `strong-2c.bml` `2C-2D-(2X)- 2Y` and
    `(3X)- 3Y`).** The expander processes a table's exact rows before its
    pattern rows (`docs/design/06-system.md` §4.2 point 2), so under
    row-order tie-breaking a pattern row loses to every exact sibling
    whatever its position in the file. A variable-suit bid (`5+#` in the
    suit named) is more specific than the neighbouring penalty double, so
    it is given a priority above it.

## Lane sayc-comp (openings, responses, rebids, competition)

Notes added while completing these five files against the replay-based consistency harness
and its coverage report (`crates/bridge-bidding/tests/consistency.rs`). They are numbered
`C1`, `C2`, ... so they cannot collide with notes other lanes append above.

C1. **Opener's rebid tables are exhaustive after a forcing response (`rebids.bml`).** A new
    suit by responder is forcing, and a two-over-one promises another bid, so opener may not
    pass. The old tables stopped at 18 hcp (a 19--21 opener had no call and was passed by the
    implicit pass) and left some 12-hcp and 16--18 hcp shapes uncovered. Every table after a
    one-level new suit or a two-over-one now splits opener's 12--21 range into the booklet's
    three bands (12--15 / 16--18 / 19--21) and ranks the calls in the booklet's order of
    preference: support for responder's major, a one-level major of opener's own, a balanced
    notrump rebid, a reverse, then a rebid of the opening suit or a non-reverse new suit. The
    19--21 band jumps: to game in a fit or in a six-card major, a jump shift with a second
    suit, or 3NT (the 19--21 hand with a long minor and no other call). A balanced 15--17
    opens 1NT and a balanced 20--21 opens 2NT, so those two bands never reach these tables,
    which is what makes the split exhaustive. After a two-over-one response, opener's 2NT is
    the balanced minimum (12--14) and 3NT the balanced 18--19; the booklet gives no numbers for
    these two, and this split follows from the 2/1's own 10+ (12--14 opposite 10+ is only
    invitational, 18--19 opposite 10+ is game). The plain rebid of the opening suit
    (`2H = 12--15 hcp` after 1!h-2!c, and so on) is the minimum "nothing else to show" call;
    every 1-of-a-major opener has 5+ cards in it, so it needs no length of its own.
    Also: the limit raise is 10--12 (was 10--11, which left 12-hcp hands with 3-card support
    and no Jacoby 2NT with no call), the 3NT response to 1M ranks above the 2/1 new suits (it
    was unreachable behind them), 2C over 1!s also covers the one 13+ shape with 3-card
    spade support and no other call (3=4=3=3), and 1!s-2!c-2!h-3!s (game-forcing jump
    preference) ranks above the fourth-suit 3!d that used to hide it.
    Responder's second call after opener's 1NT rebid (1!x-1!y-1NT) now has a full table for
    all six one-level auctions: a weak (0--10) rebid of a six-card suit or preference to
    opener's suit, a 5-card new suit that is not a reverse (non-forcing, including the weak
    5-4 1!m-1!s-1NT-2!h), 2NT or a three-level rebid/jump preference as an invitation
    (11--12), game in a six-card major or a 3-card fit for opener's major (13+), a reverse or
    jump shift as a game force, and 3NT with 13+ and nothing else. Before, most of these
    positions had only two or four rows, so invitational and game-going hands passed 1NT
    through the implicit pass.

C2. **Every common competitive position has its own table, so a hand with nothing to say
    passes instead of having no call (`competition.bml`; `harness_review.json` #0).** When
    the auction reaches a position the system does not list, `choose_bid` resolves it by
    substituting the opponents' unexpected call with a pass (`resolve_lenient`, "system
    on") and offers that position's rows; but it synthesizes the implicit pass (the
    complement of the listed rows) only at an *exact* system position, because `interpret`
    only reads a pass as that complement there (`docs/design/07-bidding.md` §5.2 step 3).
    So a hand that fits none of the substituted rows got `NoCandidate`, and the replay-based
    harness passed for it (a gap). That was the single largest source of holes: responder
    after 1!h-(1!s), after a weak jump overcall (1!d-(2!s), 1!c-(2!h)), after a two-level
    overcall and after a three-level preempt, and after a 1NT overcall. Each of these now has
    a table of its own (a cuebid limit raise or better, a single raise one card lighter than
    without interference, a preemptive jump raise, notrump with a stopper, the negative
    double through 2!s and a penalty double above it, natural forcing new suits at 11+),
    written with pattern variables where one table serves several suits: `1m-(1Y)-` and
    `1m-(2Y)-` (a minor opening, any higher overcall), `1M-(2m)-` (a major opening, a lower
    minor overcall), `1S-(3Y)-`, `1H-(3m)-`, `1m-(3Y)-` and `1X-(1N)-`; the rest are written
    out. The booklet's own numbers are used where it gives them (negative doubles through
    2!s, the cuebid as a limit raise or better, a jump raise as preemptive); the notrump
    ranges (1NT 7--10, 2NT 11--12, 3NT 13+ after an overcall; 3NT 12+ over a preempt;
    3NT is uncapped, since a stronger hand with a stopper and no fit has no other call)
    and the 10+ penalty double over a preempt are this file's interpolation.
    Responses to a double of our opening gained the single raise (6--9, 3+ for a major) and
    1NT (6--9); 2NT is now a limit raise or better *with four trumps* (it read `INV, 10+ hcp`
    with no fit, which the equal-strength redouble always outranked, so it was never bid),
    and the preemptive jump raise needs four trumps for a major and five or six for a minor
    (it required a six-card suit before, which is a jump *shift*'s length, not a raise's).

C3. **Advances, the sandwich seat, and a minimum takeout-double advance
    (`competition.bml`; `harness_review.json` #4).** Advancing a one-level overcall had only
    the cuebid row, so every other hand passed; it now has the raise (7--10, 3+ cards), the
    preemptive jump raise, a new suit, and notrump with a stopper, and the cuebid is the
    limit raise or better with a fit. After the cuebid (forcing) the overcaller has a call
    with every hand: 2NT with 13+ and a stopper, a jump in the suit with 13+ otherwise, and
    the minimum rebid of the suit. When both opponents have bid and partner has passed (the
    "sandwich" seat), the position used to be read against the balancing table as if
    responder had passed, leaving most hands without a call; it now has a takeout double of
    the two unbid suits (12+, at most three cards in either of theirs), a natural overcall
    and 1NT. These tables name the overcall's suit with whichever variable sits between
    (or below, or above) the two suits the opponents bid -- `Y` only binds above `X` and `Z`
    above `Y`, so "a suit between their two suits" is `Y` in `(1X)-P-(1Z)-` and "a suit below
    both" is `X` in `(1Y)-P-(1Z)-`. When advancing a takeout double, a weak (0--8) hand with no
    four-card unbid suit now bids the cheapest unbid suit where it holds three cards (the
    second branch of each minimum-advance row) instead of passing, which converted the
    double for penalties holding four small trumps; a pass is left only for a hand with
    length in opener's suit and no three-card unbid suit. The booklet gives no numbers for
    these advances; the ranges are this file's interpolation from the direct-seat ones.
    The opponents' 1NT opening is defended naturally (penalty double 15+, natural
    two-level overcalls with a five-card suit), which the booklet implies by describing no
    conventional defense.

C4. **Interference this file does not model reverts to natural bidding, through `(any)`
    wildcard positions with no rows (`competition.bml`).** For every table whose next call
    belongs to the opponents (the uncontested responses and rebids, the advances, the
    overcaller's rebid), an opponent's bid or double at that point was read "as if they had
    passed" (`resolve_lenient`), against the uncontested table. That table's rows were then
    partly illegal and partly inappropriate, most hands fit none of them, and there is no
    implicit pass at a lenient match, so `choose_bid` returned `NoCandidate`; each table added
    for the uncontested auction made this worse. The booklet's own rule is that
    interference cancels the partnership's agreements and bids become natural, so the end
    of `competition.bml` lists these positions as history lines ending in `(any)` (the
    `OppClass::AnyCall` wildcard, an extension token) with no rows under them. The trie
    follows the exact edge (the pass that the table after it describes) when there is one
    and the wildcard otherwise; a position with no rows is off-system, so the next call is
    chosen by natural inference, exactly as `interpret` reads it. On the 10^6-position
    harness this took `NoCandidate` from 22,583 to 7,619 in one step.

C5. **Positions where natural inference had nothing to offer are written out.** At a
    position with no table, the next call is chosen by natural inference; its pass rule
    covers only a weak hand, so a hand of middling strength with no natural call had no
    candidate at all (`NoCandidate`), the most frequent kind of hole left after C4. The
    commonest such positions in these files now have tables of their own, so their implicit
    pass is the complement of the rows and every hand has a call:
    advancing a natural two-level overcall of the opponents' 1NT (a raise with 8--11 and a
    fit, game with 12+, a six-card signoff, a raise or a penalty double if opener's partner
    competes). Two response holes over a minor opening are closed as well: the invitational
    3=3=3=4 hand over 1!c raises to 3!c with four clubs (the booklet's "one fewer in a
    pinch"), and a hand above 3NT's 16--18 with no four-card major bids 3NT instead of passing
    (the booklet has no forcing minor raise to start with instead). All ranges here are this
    file's interpolation; the booklet gives none.
    Opener's rebid after responder's forcing new suit over an overcall had no table either:
    after a one-level response it is the uncontested rebid table for the same two suits,
    shared through `#COPY`/`#PASTE` (the overcall only removes calls those tables never
    used), and after a two-level response (11+, five cards) a small table pasted per auction:
    a raise, 2NT 12--14 or 3NT 18--21 with a stopper, and otherwise the cheapest rebid of the
    opening suit. Each new table's position also gets an `(any)` line (C4), since otherwise
    a further bid by the opponents would be read against the new table as if they had passed.

C6. **More positions natural inference could not answer (`competition.bml`, `rebids.bml`).**
    Continuing C5 down the frequency-ranked `NoCandidate` list: opener after responder's
    game-forcing fourth suit (1!s-2!c-2!h-3!d: a second five-card suit, 3NT with a diamond
    stopper, three-card club support, else 3!s) and jump preference (1!s-2!c-2!h-3!s: game
    with 12--15, a 4!c slam try with more); advancer after opener's partner bids over the
    overcall (a raise with a fit, at the three level when the cheap raise is gone, a
    preemptive jump with four trumps, the cuebid with 11+ below the overcall's suit, a
    penalty double of 2NT); advancer after a 1NT overcall is taken out (3NT with 10+, a
    penalty double with 8--9); a raise after a negative double of a two-level overcall;
    advancer's choice of major after a minor-suit Michaels cuebid (game 13+, a jump with
    11--12, spades with three or more and a doubleton heart or four spades and three hearts,
    hearts otherwise, 3NT with 13+ and no major fit); and a raise of a sandwich-seat
    overcall when the opponents bid again. As before, the ranges are interpolations: the
    booklet describes none of these auctions.
    Also: a natural two-level new suit when advancing a one-level overcall (10+, five
    cards, not three-card support), opener after the fourth suit in 1!h-1!s-2!c-2!d, and
    responder after a 3!c preempt over 1!d (the `1m-(3Y)-` table only binds suits above the
    opening, so this one is written out).

C7. **Balancing over the opponents' 1NT is played like the direct seat (`competition.bml`).**
    `(1X)-P-(P)-` binds a suit, so `(1N)-P-(P)-` had no table: the balancing overcall was
    bid by natural inference and advancer had no table at all, which made
    `(P)-1N-(P)-P-2!d-(P)` the most frequent `NoCandidate` position of the integration
    10^6 consistency run (505 of 5037 positions, middling hands with a fit or game
    values). The balancing seat now has the direct seat's penalty double and natural
    two-level overcall, with the same advances (raise 8--11, 4M or 3NT with 12+, a
    six-card signoff, a raise or penalty double if opener's partner competes) and an
    `(any)` line.
    Lenient matching reads an uncovered call of the opponents as a pass, so without
    more the new `(1N)-P-(P)-` table also captured our seat after *their* 1NT
    responses (1NT-P-2!d transfer, 1NT-P-2!c Stayman), where its double and two-level
    overcalls are mostly illegal or wrong: `NoCandidate` rose from 2744 to 7673 in the
    same run. An empty `(1N)-P-(any)-` line makes those positions off-system (natural
    bidding), as C4 does for the suit openings.

C8. **Phase-3 recheck: rows that caught every hand, or were shadowed by a sibling.**
    - Takeout double (`competition.bml`): `(1X)- D = 12+ hcp` had no shape, so a
      12--16 hand with five cards in the opener's suit doubled for takeout. It is now
      `12--16 hcp and 0--2X or 17+ hcp`; the balancing double is `8--16 hcp and 0--2X
      or 17+ hcp`. A hand with length in their suit and no overcall passes.
    - Advancing a takeout double: the minimum rows had no HCP cap and outranked the
      cuebid, so game-forcing hands made a minimum bid. The minimum rows' 4+ branch is
      now 0--11, the jumps 9--11 with a 5+ card suit, and the cuebid `{prio:10} F,
      12+ hcp` outranks both tiers. Within a tier a major now outranks a minor (over
      1!c a 4-4 minor/major advancer used to bid 1!d).
    - Michaels against our opening: `1M-(2M)- 3oM = F` and `1m-(2m)- 3M = F, 4+M` had
      no strength, so every hand under 10 hcp made the game-forcing cuebid. The cuebids
      now need 13+ (with support for opener's suit after a major Michaels) and outrank
      the 10+ double; a 6--10 simple raise is added and weaker hands pass. Advancer
      after `(1H)-2H-(P)` / `(1S)-2S-(P)` used to have only `2N = F`; the known major is
      now supported by strength (cheapest bid, jump 11--12, game 13+) and 2NT needs a
      doubleton or less in it.
    - Balancing jump overcall: `6+#` (see #23).
    - `1N-(2X)-` three-level new suits (`notrump.bml`, #21) were `NAT` only, so a
      1-hcp hand with a five-card suit bid at the three level. They are now `{prio:1} F,
      10+ hcp, 5+`: forcing, and preferred to the suitless `3X` cuebid, which would
      otherwise win every such hand by row order. Weaker hands compete at the two
      level (`2Y = NAT`) or pass.
    - `1M-4M` (`responses-major.bml`): the shutout raise shared the default priority
      with the single raise, which came first, so it was reachable only at 0--5 hcp.
      It now has `{prio:1}`.
    - Over 1!c/1!d (`responses-minor.bml`) 1!h came first, so 5-4 and 5-5 hands with
      the spades at least as long bid 1!h. 1!h now excludes those length pairs (the
      openings' length-pair encoding), so they bid 1!s; up the line still applies to
      4-4.
    - `2C-2D-` (`strong-2c.bml`): a 22+ hand with no five-card suit that was not
      balanced (4-4-4-1), and every 25+ hand with no five-card suit, had no rebid and
      passed the forcing 2!d. 2NT now also takes 22--24 hands with no five-card suit,
      and 3NT takes 25+ hands with no five-card suit (or balanced).

## Phase 4 tables (tasks 4.2-4.4)

Phase 4 measures SAYC on auctions it generates itself (`cargo xtask coverage`, below): a
position without rows is answered by natural inference, and an auction is *all-system* only
when no call needed it. The booklet stops at the first round or two, so most of the phase-4
text is this file's own limit-bidding interpolation, written per auction; none of it adds a
convention. Every table ends with the partnership passing from then on.

P1. **System stops** (`passes.bml`; docs/design/06-system.md §4.5). Our own pass is a trie
    edge only where a row names it, so a partnership that stopped used to leave the system at
    the partner's next turn. Phase 4 first wrote the stop out: the clipboards `pass-chain`
    (our pass, then `(any)` of theirs, six rounds deep) and `after-chain` (starting with their
    call, pasted under a row whose call ends our bidding), 2,543 pastes and about 87% of the
    38,737 rows. They are now replaced by stop markers at the same places:
    `P = {prio:-100} {stop} any hand` for each `#PASTE pass-chain` (1,146) and `#STOP` at the
    paste's indentation for each `#PASTE after-chain` (1,397). The compiler grafts every stop
    onto the trie: from the stop it follows the opponents' `(any)` and our `P` alternately,
    through any edge a table writes there itself, and links the first missing edge to one
    shared pair of nodes that loop. The stop pass is an ordinary `{prio:-100}` `any hand` node,
    so under the phase-4 rank policy it is still chosen exactly when no other listed call
    applies, and every consumer sees what it saw of the chain rows. Unlike the chains a stop
    never runs out (the chains stopped after six rounds; with four the generated all-system
    rate fell from 0.883 to 0.824, with three to 0.614). With the graft cut at six rounds the
    stop-based file is indistinguishable from the chains (every `cargo xtask coverage` number,
    and `choose_bid`, `call_distribution` and `interpret` on 24,269 generated positions);
    unbounded, only positions past the sixth round differ.
    **Caveat (review of lane D), unchanged by the stops.** Once one of us has passed, the
    partner passes with *any* hand whatever the opponents do, and the stop pass is the only
    row of those positions. They count as system positions, so the stop also suppresses the
    natural completion that would mark them as holes: without chains or stops the generated
    all-system rate is 0.044, not 0.896. At many of them SAYC has a real decision (a
    reopening double, competing after a negative double and their raise, a penalty double of
    a balancing bid). `cargo xtask coverage` therefore reports the *strict* rate as well: a
    position whose only rows are default passes (priority -100 or lower, the stop pass
    included) counts as a departure whenever the natural choice there is not `Pass` (see P4'
    below). The fitted `(ε, δ)` depends on the stops too (without them ε 0.361, δ 0.412). A
    chain that continued only over the opponents' pass (`(P)` for `(any)`) was measured and
    rejected: all-system 0.333, because the partnership then leaves the system in every
    auction the opponents keep bidding in, mostly where the natural choice is a pass as well.
P2. **Defense to their two-level and higher openings** (`defense.bml`): the one-level methods
    one level higher (takeout double short in their suit, 12--16 or any 17+; a natural
    overcall with five cards at the two level and six at the three level; 2NT 15--18 with a
    stopper over a weak two; 3NT to play), with the advances of the double and of the
    overcall. Over a four-level preempt double with 16+, else pass; over their strong 2!c and
    2NT a natural overcall needs a good hand and a long suit.
P3. **Continuations after our pass** (`continuations.bml`): the stop pass
    (`P = {prio:-100} {stop} any hand`, the `pass-chain` paste before the system stops) as the
    lowest-ranked call of every table that has no pass of its own, and opener's reopening
    after an overcall and responder's pass (double short in their suit with 12+, rebid a
    six-card suit; responder then passes for penalty with four cards in their suit, bids
    notrump with a stopper, or returns to opener's suit).
P4. **Opener after a negative double** (`competitive-rebids.bml`): the unbid four-card major at
    the cheapest level with 12--15, a jump with 16--18, game with 19--21; notrump with a
    stopper; else the cheapest rebid of the opening suit (the double is forcing). Responder
    raises to game with 12+, invites with 10--11, and so on.
P5. **Later uncontested rounds** (`later-rounds.bml`, the weak-two 2NT inquiry in
    `weak-twos.bml`): responder adds opener's shown range to his own and bids game with enough
    for 25--26, invites a point or two below, else stops; opener accepts an invitation with
    the top of his range. An eight-card major fit plays in the major, anything else in
    notrump. Neither slam bidding nor a second-round forcing new suit is written.
P6. **Later competitive rounds** (`competitive-later.bml`): opener's answer to a negative
    double of a two-level overcall, opener after responder's raise or notrump over an
    overcall, the overcaller's side after the advance, two-level and weak jump overcalls,
    Michaels and the unusual notrump, the balancing seat, and the takeout doubler's rebid.
P7. **Further competitive continuations** (`competitive-extra.bml`, the doubled preempt in
    `preempts.bml`): tables generated once from the most frequent departures of a 5000-replay
    coverage run and checked against the compiled system (every one of our header calls an
    unshadowed sibling, the position still empty); each table gets an `(any)` sibling so
    that `resolve_lenient` does not read their double or bid as a pass (the corpus lenient
    count had risen from 5 to 66 of 8169 calls and is 1 now). The calls are natural: a raise
    with the stated support at the cheapest level, a rebid of a six-card suit, notrump with a
    stopper.
P8. **Advances after they raise over Michaels, and of the sandwich overcall over their 1NT
    response** (end of `competitive-later.bml`): over a minor-suit cuebid game in a major with
    11+ and three-card support, else the cheapest major with support; over a major-suit
    cuebid game in the known major (11+ over 2!h, 8+ over 2!s), else 3!s with 0--10 over 2!h;
    over `(1X)-P-(1N)-2Y` and opener's pass a raise with 8--11 and three-card support. These
    were the most frequent off-system `NoCandidate` positions of the 10^6 forward-consistency
    generator after P1-P7 (for example `(1S)-P-(1NT)-2H` advancer 73 and `(1S)-2S-(3S)`
    advancer 67 per 10^6). Also the advancer after our balancing two-level overcall of their
    raise and opener's pass (`(1X)-P-(2X)-2Y-`, a raise with 8--11 and three-card support).
P9. **Advancing our one-level overcall after a negative double** (`competition.bml`, the
    overcaller's continuations at the end of `competitive-later.bml`). The phase-3 header
    `(1X)-1Y-(D)-2X-(P)-` named advancer's cuebid with no row behind it, so at `(1X)-1Y-(D)`
    the cuebid was an unconstrained trie edge: `choose_bid` cuebid with any hand and never
    passed, and the implicit pass had an empty complement (the corpus's one default-mode
    `EmptySupport` seat, `P P 1C 1S X P ...`). The advance is now the table over opener's
    partner's pass (the double changes nothing in SAYC), with the two-level new suit and the
    pass chain, and the overcaller's continuations after it are copies of the `(P)` ones
    (raise, jump raise, new suit, notrump; 70 tables, generated from the `(P)` headers and
    checked to name only calls the advance table defines). `cargo xtask coverage` now counts
    our own non-pass calls without a requirement (`lints.unconstrained_own_calls`); it is 0.

P10. **Competitive decisions where the chain pass was the only row** (end of
    `continuations.bml`): opener after a negative double of a two-level overcall and their
    raise (support for the doubler's major, game with 17+; over their raise of a major, a
    four-card minor at the four level with 15+; a six-card rebid with 15+); opener's
    reopening over a three-level overcall (takeout double with 12+ and 0--1 cards in their
    suit, a six-card rebid with 15+; responder passes for penalty with four trumps, bids
    3NT with a stopper, else returns to opener's suit); opener after their takeout double,
    responder's pass and advancer's 1NT (penalty double 16+, a six-card rebid 12--15);
    opener after responder's pass and their raise of a one-level overcall (takeout double
    13+ short in their suit, answered in an unbid major, else opener's suit; a six-card
    rebid 15+); responder's penalty double with 10+ when a passed hand balances over
    1NT-3NT at the four level. These were the reviewer's examples of strong hands the
    chains made pass (`sayc_content::phase4_tables::acting_after_they_compete_over_our_stop`).
    The strict all-system rate rises from 0.540 to 0.550; the positions left are a long tail
    (the top one, opener after `1S-(P)-P-(X)`, has 9 of 461 overrides in 1000 auctions), and
    many overrides are the natural engine competing with hands SAYC passes with.

Lints: the phase-4 rows add no `ShadowedBranch` warning on our side (18 before and after,
all phase-3 rows). They add 129 on the opponents' side, every one on a table-header node:
a header such as `1C-(1D)-1H-(1N)-` names their call with no constraint, next to the row that
defines that call (here `competition.bml`'s advance of the overcall), so the header node is a
second, lower-ranked member with the same call and is never the first satisfied one. The call
itself keeps its pieces; the base system has 231 lints of exactly this kind. P10's headers add
22 more of the same kind (theirs 382, ours still 18). Opponents' calls are trie edges only, so
the lint now skips `Side::Them` nodes (phase-4 integration): SAYC reports `ShadowedBranch` 18,
all on our side, and `OverlappingBranches` 268. The "theirs" counts below predate that change.

Cost: with the pass chains the system grew from 1,611 rows / 2,415 nodes to 38,737 rows /
49,800 nodes (the chains about 87% of them); release compile 0.91--0.94 s best of 3 and
`tests/compile_time.rs`' release-only `< 1 s` assertion on `sayc.bml` failed on some runs
(1.03--1.49 s at loadavg 3.3--5.9); the exclusive index took
44.7--47.5 ms to rebuild (24,067 groups); the postcard IR was 16,415,084 bytes (845,860 before
phase 4); one compile peaked at 158 MB RSS. With the system stops (phase-4 integration,
stage 2; release, best of 3, loadavg 3.2--3.5): 6,496 rows / 7,174 nodes (2 of them the
synthesised stop pair), compile 437--441 ms, `compiling_sayc_is_fast` 436--474 ms, index
rebuild 12.8--13.0 ms (2,754 groups; 21.8 ms before three build-time shortcuts that leave the
index byte-identical), postcard IR 2,524,018 bytes, one compile peaks at 33 MB RSS, and a
default-sizing `cargo xtask coverage` run at 65 MB (241 MB with the chains).

## Phase 4 coverage (`cargo xtask coverage`)

`cargo xtask coverage` (`xtask/src/coverage.rs`) writes `target/coverage_report.json`; the
fields are described in its module doc. The numbers below are the rows this file is
measured against in phase 4 (docs/design/15-phase4-plan.md, lane D; 12-roadmap tasks
4.1-4.4). "Positions" is the forward-consistency generator (seed `0x5a1c0002`, 5% random
calls), run here with `COVERAGE_POSITIONS=1000000`; the other sections use their defaults.

P0. **Baseline, before any phase-4 SAYC change** (wip/p4-api 4b131db; release; 52.5 s,
    loadavg 9.27 -> 10.22; positions 49.4 s of it).
    - Lints: Error 0, Warning 1167, Info 3009. Exclusive index: 714 groups, 2270 nodes,
      2794 branches; 238 calls and 249 branches are never chosen (shadowed) in any group
      they appear in.
    - Generated (1000 replays with natural completion, seed `0xC0FE4001`): all-system
      27/1000 (0.027); 973 auctions contain a natural completion, 26 a gap. Calls: system
      3183, system implicit pass 1602, natural 12490, gap 37. First departure from the
      system: our own pass has no trie edge 385, the system is exhausted (no legal
      child) 198, no rows for their opening (weak twos, preempts) 184, their call not in
      the trie 127, their pass 79. Final contract level `[passout, 1..7]`:
      `[14, 21, 94, 143, 79, 29, 9, 611]` (the natural escalation fixed by the level
      floor, lane S).
    - Positions (10^6): chosen by the system 269,948, natural 581,628, implicit pass
      144,309, `NoCandidate` 4,115 (410 of them on-system, all at lenient matches). The
      phase-3 tops, keyed like the phase-3 report (trie position + role): `1D-(3C)`
      responder 27, `1D-(1H)` responder 22, `1C-(1H)` responder 15 (the roadmap's
      36 / 31 / 25 were measured before the phase-3 rechecks).
    - Corpus (724 auctions: 27 files, PBN then LIN; even index = tune, odd = eval):
      all-Exact 30/724 (0.041), eval 14/362 (0.039), SAYC-compatible-opening subset
      13/384 (0.034). Call-level system resolution (Exact or Partial): all 0.405, eval
      0.409, subset 0.424. `resolve_lenient` used by 5 of 8169 calls (0.0006). Seats with
      empty strict support 30; seats whose default-mode support is empty 1.
    - True-deal policy agreement (human call == `choose_bid`'s, or the natural choice
      off-system): system positions 2090/3485 (0.600), natural positions 2077/4684
      (0.443); eval split 0.616 / 0.456.
    - `(ε, δ)` MLE on the tune split (4135 calls): ε = 0.490, δ = 0.309, ln L = -9351.0
      (-2.261 per call); at the placeholder `human()` (0.01, 0.3) ln L = -15390.1, at
      `system_players()` -21012.9. Eval split at the MLE: -8801.9 over 4034 calls.

P1'. **The same base SAYC under the phase-4 engine** (the base files of P0, compiled and
    replayed with this branch's code, i.e. after the wip/p4-S merge: natural level floor,
    SAYC-shaped natural rules; release, `COVERAGE_POSITIONS=1000000`, loadavg 8.2). It
    separates the effect of the rows below from the engine's: generated all-system 27/1000;
    positions `NoCandidate` 8,826 per 10^6 (on-system 203), phase-3 tops `1D-(3C)` 19,
    `1D-(1H)` 146, `1C-(1H)` 101; corpus all-Exact 0.041 / eval 0.039 / subset 0.034,
    system resolution 0.405 / 0.409 / 0.424, `resolve_lenient` 5 of 8169 calls; agreement
    0.600 (system) / 0.627 (natural); MLE ε = 0.375, δ = 0.460. ShadowedBranch 249 (ours 18,
    theirs 231, all on table headers).

P2'. **After P1-P8** (this branch; release; `COVERAGE_POSITIONS=1000000`, 17.0 s at loadavg
    3.5; the default sizing takes 5.8 s):
    - Compile 855 ms (35,856 rows, 45,749 nodes); lints Error 0, ShadowedBranch 378 (ours 18,
      theirs 360; the 129 new ones are table headers, see "Lints" above); exclusive index
      22,088 groups, fresh build 43 ms best of 3; postcard IR 15,118,747 bytes.
    - Generated (seed `0xC0FE4001`): all-system **894/1000 (0.894)**; 104 auctions with a
      natural completion, 2 with a gap. Calls: system 7515 (4125 of them default chain
      passes), system implicit pass 938, natural 214, gap 2. First departures: their pass not
      in the trie 74, the system exhausted 18, their call not in the trie 14. Final contract
      level `[passout, 1..7]`: `[14, 81, 324, 436, 141, 3, 1, 0]`. Held-out seeds (2000
      replays each): `0x1234` 0.895, `0xBEEF0001` 0.875, `0xD00D` 0.8755.
    - Positions (10^6): `NoCandidate` 3,069 (on-system 138); the phase-3 tops `1D-(3C)`,
      `1D-(1H)` and `1C-(1H)` responder are 0 / 0 / 0. The remaining tops are off-system
      positions after the generator's random jumps (`1D-(5D)` responder 36 per 10^6) and a
      few advances; the natural implicit pass (lane B) answers most of them.
    - Forward consistency (release, seed `0x5a1c0002`): 10^5 positions 0 non-gap violations,
      31 gap-induced; 10^6 0 non-gap, 401 gap-induced (phase 3: 2,645).
    - Corpus (724 auctions): all-Exact 206/724 (0.285), eval 105/362 (0.290), subset
      112/384 (0.292). System resolution (Exact or Partial): all 0.632, eval 0.654, subset
      **0.653** (target 0.80), subset eval 0.681. `resolve_lenient` 1 of 8169 calls. Seats
      with empty strict support 27; sampler `EmptySupport` 1 (a tune-split auction). In the
      subset, 273 of 384 auctions leave the system; the first natural call is a recorded call
      that is not a row at an on-system position in 221 of them (`call_not_a_row`: the
      players' own methods, for example 1!c-2!d, or a 2/1 continuation), a hole of ours in 52
      (their pass not in the trie 19, the system exhausted 18, our pass 9, their call 5), and
      875 of the subset's 1511 natural calls follow an earlier call of ours that was off the
      system.
    - True-deal agreement: system positions 3851/5739 (0.671), natural positions 1438/2430
      (0.592); eval 0.688 / 0.599.
    - MLE on the tune split (4135 calls): **ε = 0.3373, δ = 0.3516**, ln L = -7347.0 (-1.777
      per call); profile 95% intervals ε 0.322--0.349, δ 0.31--0.395. ln L at δ = 0 / 0.1 /
      0.2 / 0.3 / 0.4 / 0.5 (ε at the MLE): -7722.1 / -7440.6 / -7373.6 / -7349.7 / -7349.2 /
      -7367.0; at ε = 0.001 / 0.01 / 0.1 / 0.2 / 0.3 / 0.4 / 0.5 (δ at the MLE): -13800.7 /
      -10858.0 / -8134.4 / -7541.6 / -7359.0 / -7378.7 / -7548.9. The placeholder `human()`
      (0.01, 0.3) gives -10866.0, `system_players()` -15412.7. Eval split at the MLE:
      -6718.9 over 4034 calls.

P3'. **After P9** (this branch; release; `COVERAGE_POSITIONS=1000000`, 17.3 s at loadavg 5.6):
    - Compile 909 ms (37,535 rows, 47,611 nodes); lints Error 0, ShadowedBranch 378 (ours 18,
      theirs 360, unchanged by P9), unconstrained own calls 0 (6 before P9, all from the
      `(1X)-1Y-(D)-2X-(P)-` header); exclusive index 22,999 groups, fresh build 44.6 ms best of
      3; postcard IR 15,768,137 bytes.
    - Generated (seed `0xC0FE4001`): all-system **894/1000 (0.894)** (0.869 with only the P9
      advance table, before its continuations); 104 with a natural completion, 2 with a gap;
      calls system 7443 (4112 default chain passes), system implicit pass 938, natural 214,
      gap 2. Final contract level `[passout, 1..7]`: `[14, 92, 307, 449, 134, 3, 1, 0]`.
    - Positions (10^6): `NoCandidate` 2,995 (on-system 143); phase-3 tops 0 / 0 / 0.
    - Forward consistency (release, seed `0x5a1c0002`): 10^5 0 non-gap / 36 gap-induced
      (3.3 s, loadavg 4.6); 10^6 0 non-gap / 424 gap-induced (20.6 s, loadavg 4.6 -> 4.1).
    - Corpus: all-Exact 211/724 (0.291), eval 107/362 (0.296), subset 116/384 (0.302).
      System resolution all 0.639, eval 0.659, subset **0.661** (target 0.80), subset eval
      0.688. `resolve_lenient` 1 of 8169 calls. Seats with empty strict support 15 (27
      before P9); sampler `EmptySupport` **0** (1 before). Subset first natural calls:
      `call_not_a_row` 226, the system exhausted 18, their pass 19, their call 5 (our pass 0,
      9 before).
    - True-deal agreement: system positions 3906/5794 (0.674), natural 1397/2375 (0.588); eval
      0.690 / 0.597.
    - MLE on the tune split (4135 calls): **ε = 0.3357, δ = 0.341**, ln L = -7330.8 (-1.773
      per call); δ within 1.92 of the maximum for 0.30--0.38, ε between the grid points 0.316
      and 0.355. ln L at δ = 0 / 0.1 / 0.2 / 0.3 / 0.4 / 0.5 (ε at the MLE): -7686.5 / -7416.8 /
      -7353.8 / -7332.5 / -7334.1 / -7353.7; at ε = 0.001 / 0.01 / 0.1 / 0.224 / 0.316 /
      0.355 / 0.501 (δ at the MLE): -13772.7 / -10834.5 / -8115.5 / -7456.2 / -7334.3 /
      -7333.6 / -7537.1. The placeholder `human()` (0.01, 0.3) gives -10840.5,
      `system_players()` -15326.0. Eval split at the MLE: -6706.5 over 4034 calls.

P4'. **After P10, with the strict accounting** (this branch; release; `COVERAGE_POSITIONS=1000000`,
    17.6 s at loadavg 2.6 -> 2.9):
    - Compile 957--983 ms (38,737 rows, 49,800 nodes; loadavg about 5); lints Error 0,
      ShadowedBranch ours 18, theirs 382; unconstrained own calls 0; exclusive index 24,067
      groups, fresh build 45.7--49.2 ms; postcard IR 16,438,261 bytes; peak RSS of a default
      coverage run 241 MB. `tests/compile_time.rs`' release `< 1 s` on `sayc.bml` now fails
      (1.24 s alone, 1.42 s with the other tests; loadavg 3.8--4.6).
    - Generated (seed `0xC0FE4001`): all-system 894/1000 (0.894), **strict 550/1000 (0.550)**
      (0.540 before P10). 2,706 positions offered only default passes; at 461 of them the
      natural choice was a call (366 auctions, 344 of them otherwise all-system). Strict first
      departures: default-pass override 357, their pass not in the trie 61, the system
      exhausted 18, their call not in the trie 14. Final contract level `[passout, 1..7]`:
      `[14, 92, 304, 450, 136, 3, 1, 0]`.
    - Positions (10^6): `NoCandidate` 3,009 (on-system 142); phase-3 tops 0 / 0 / 0.
      Forward consistency (release, seed `0x5a1c0002`): 10^5 0 non-gap / 26 gap-induced (3.4 s,
      loadavg 4.0); 10^6 0 non-gap / 426 gap-induced (20.1 s, loadavg 4.4 -> 4.2).
    - Corpus: all-Exact 0.305 / eval 0.309 / subset 0.326; system resolution 0.641 / 0.661 /
      **0.665** (subset eval 0.692); `resolve_lenient` 1 of 8169 calls; strict-empty seats 18;
      sampler `EmptySupport` 0. Subset first natural calls: `call_not_a_row` 137,
      `call_not_a_row_default_pass_only` 80 (91 before P10: a recorded call at a position the
      system answers only with a chain pass, i.e. a SAYC decision the file does not write,
      not a method of the players), their pass 19, the system exhausted 18, their call 5.
    - Agreement: system positions 0.675, natural 0.588 (eval 0.691 / 0.597).
    - MLE on the tune split: **ε = 0.3357, δ = 0.335**, ln L = -7320.7 (-1.770 per call); δ
      within 1.92 of the maximum for 0.30--0.38. ln L at δ = 0 / 0.1 / 0.2 / 0.3 / 0.4 / 0.5:
      -7666.3 / -7402.7 / -7341.8 / -7321.9 / -7324.6 / -7345.2; at ε = 0.001 / 0.01 / 0.1 /
      0.2 / 0.316 / 0.398 / 0.501: -13750.6 / -10817.0 / -8102.6 / -7514.2 / -7324.0 / -7351.6
      / -7528.2. `human()` placeholder -10822.0, `system_players()` -15274.7; eval split at
      the MLE -6707.5.
    - The same files with every chain paste removed (release, default sizing, loadavg 3.2):
      5,010 rows / 5,499 nodes, compile 396 ms, index 17.9 ms, IR 1,913,553 bytes;
      all-system 0.044 (strict 0.044); corpus all-Exact 0.047 / 0.047 / 0.044, system
      resolution 0.443 / 0.450 / 0.463; MLE ε 0.361, δ 0.412.

P11. **Phase-4 integration: the chains replaced by system stops** (P1; default sizing,
    release). Stage 1 (0928a7b, the merged lanes with the chains; loadavg 6.2) against
    stage 2 (the stops; loadavg 6.9--7.4); every difference comes from the stop not running
    out after six rounds (with the graft cut at six rounds every number below is identical to
    stage 1):
    - System: 38,737 rows / 49,800 nodes -> 6,496 / 7,174; compile 1028 -> 471 ms (single
      run inside the coverage tool); index 24,067 -> 2,754 groups, fresh build 47.5 -> 12.5 ms;
      IR 16,415,084 -> 2,524,018 bytes; peak RSS of the whole run 65 MB. Lints:
      `EmptyDescription` 23,017 -> 1,703, `NonStandardToken` 37,998 -> 734, warning
      `SiblingSubset` 2,318 -> 2,271, `ShadowedBranch` 18 (all ours) and `OverlappingBranches`
      268 unchanged, `DuplicatePath` 5 (the placeholders the stop pass fills).
    - Generated: all-system 0.894 -> 0.896, strict 0.550 -> 0.552; default-pass-only
      positions 2,706 -> 2,710 with 461 overrides both times; 2,463 calls are the synthesised
      stop pass (`generated.system_stop_passes`).
    - Corpus: all-Exact 0.305 / 0.309 / 0.326 unchanged; system resolution 0.641 -> 0.658,
      eval 0.661 -> 0.675, subset 0.665 -> 0.683 (subset eval 0.692 -> 0.707); subset first
      natural calls unchanged (`call_not_a_row` 137, `..._default_pass_only` 80); subset
      natural calls because their call / their pass is not in the trie 154 -> 85 / 99 -> 85.
      MLE ε 0.3483 unchanged, δ 0.3253 -> 0.3246, ln L -7418.7 -> -7416.2.
      The +0.016 to +0.019 of system resolution is entirely stop passes: +133 resolved calls,
      all human passes at default-pass-only positions (rounds 3-6). It is not authored
      coverage, so the [C] baseline is not restated upwards. Strict [C] (an Exact or Partial
      call where the caller's system offers only default passes does not count;
      `system_resolution_strict_rate`, `resolved_at_default_pass`) is identical at both
      stages: all 0.442, eval 0.449, tune 0.436, subset 0.462, subset eval 0.473. The 0.80
      criterion applies to the strict value.
    - Forward consistency (release, seed `0x5a1c0002`): 10^5 0 non-gap / 2 gap-induced,
      10^6 0 / 14, `NoCandidate` 149 per 10^6, the same at stage 1 (0928a7b): the stops did
      not change them. The improvement over lane D before the merge (wip/p4-D: 26, 426,
      3,009) came from merging the other lanes.
    - Review fixes (stage 3; the SAYC trie and nodes are unchanged apart from the synthesised
      nodes' flag and text; IR 2,531,202 bytes): stops under different `#SEAT`/`#VUL`
      conditions meeting at one edge now share a loop of the union (before, only the first
      condition kept the stop pass; SAYC has no such conditions); written rows and wildcard
      edges take precedence over a stop whatever the file order (documented, tested);
      `{ stop }` is a stop; `Node::is_synthesised()` reads `NodeFlags::synthesised`;
      explanations drop the `{prio}`/`{w}`/`{stop}` annotations, so every stop pass explains
      itself as `any hand`.

P12. **Lane D2: thickening SAYC and extending BML** (wip/p4-D2; default sizing, release;
    `COVERAGE_OUT` per batch). Ten batches of rows (`continuations-p12.bml`,
    `later-rounds-extra.bml`, `competitive-extra.bml`, `competitive-later.bml`,
    `continuations.bml`) and three BML extensions, measured against the stage-2 stops of
    P11 with the strict accounting. Criteria: strict [G] >= 0.80 and strict [C] subset
    >= 0.80. **Neither is met**: strict [G] 0.552 -> **0.751**, strict [C] subset 0.462 ->
    **0.489**.
    - Survey (batch 7's dump; the classes are (i) a SAYC decision the file does not write,
      (ii) a SAYC decision BML cannot express, (iii) our passed hand in the opponents'
      constructive auction, (iv) a method of the players or an off-system position, (v) a
      convention SAYC has and the file lacks, (vi) other; a heuristic classification by
      position and call, checked by hand on the tops):
      [G] `default_pass_override_top50` 82 overrides: (i) 42, (iii) 33, (iv) 7;
      `first_strict_departure_top50` 80: (i) 48, (ii) 3, (iii) 23, (iv) 6;
      `natural_completion_top50` 64: (i) 43, (ii) 10, (iv) 11.
      [C] `first_natural_top50` 109: (i) 90, (iii) 11, (iv) 8. The subset's 4,355 calls:
      system 2,105, natural 1,261 ((i) 874, (ii) 123, (iii) 88, (iv) 95, (v) 81), resolved
      only by a default pass 989 ((i) 328, (iii) 604, (iv) 57). By class: they opened and we
      passed, default pass only, 604; we opened, uncontested, natural 365; we opened,
      contested, natural 416; they opened and we acted, natural 357.
      Class (ii) led to the extensions below; class (v) is mostly Blackwood responses, which
      need an ace-count vocabulary (the description language has no ace metric), not added.
    - BML extensions (06-system.md §4.6, §4.7, §7.4; 13-decisions.md D16 amendment items
      1-4): relative levels `cS`/`jY`/`cN` (the cheapest sufficient level of a strain and
      one above it; `LevelWithoutAnchor` Error, `NoSufficientLevel` Info); suit-length
      comparisons in descriptions (`!s>=!h`, `M>oM`, `!h>!s`); the `#ANYORDER` table
      directive (the table's fresh X/Y/Z drop the X<Y<Z order; `AnyOrderWithoutVariables`
      Info). `COMPILE_REVISION` 7. P10's twelve literal negative-double tables and six
      reopening tables became five variable tables with an identical compiled node set
      (8,834 positions compared field by field).
    - Rows by batch: 1 competing after their raise and over passed preempts (relative
      levels); 2 opener's second turn when they come in again; 3 over their notrump and
      weak-two responses, escapes; 4 advancing after their bid over our double or overcall;
      5 responder's second call and opener's rebids over overcalls; 6 competitive decisions
      after they raise or reopen; 7 continuations after rows that stopped short (Jacoby 2NT,
      weak twos, preference, cue-bid raise); 8 uncontested rebids (1X-1Y-1Z-1NT, 1m-1NT-2m,
      reverses, jump shifts), penalty sits (weak two, 1NT overcall) and takeout doubles of
      three-level preempts; 9 advancing a raised weak jump overcall and a takeout double
      over their jump raise, opener after our double of their Michaels cue bid and after a
      two-over-one and their overcall, passes that end limited auctions; 10 opener passes
      partner's game sign-off (12--15; hands with slam values have no row there and go to
      the natural engine). Every explicit pass describes the hands that pass (a range the
      earlier bidding leaves room above or below, or a shape); no row is `any hand`. A
      position an earlier file already ends with `#STOP` keeps that stop pass: an explicit
      pass written later is a `DuplicatePath` and never chosen (batch 9 removed 18 such).
    - Curve (strict [G] / raw [G] / strict [C] subset / [C] subset / nodes / compile /
      index / loadavg at the start of the run; strict [C] exists from the p4int merge on):

      | batch | strict [G] | raw [G] | strict [C] sub | [C] sub | nodes | compile | index | loadavg |
      | --- | --- | --- | --- | --- | --- | --- | --- | --- |
      | 0 (P11 stage 2) | 0.552 | 0.896 | 0.462 | 0.683 | 6,477 | 456 ms | 12.6 ms | 4.1 |
      | 1 | 0.582 | 0.896 | — | 0.685 | 6,770 | 495 ms | 13.8 ms | 3.7 |
      | 2 | 0.637 | 0.896 | — | 0.691 | 7,231 | 489 ms | 14.7 ms | 3.6 |
      | 3 | 0.663 | 0.897 | — | 0.692 | 7,580 | 678 ms | 19.5 ms | 5.4 |
      | 4 | 0.665 | 0.901 | — | 0.700 | 7,854 | 558 ms | 16.0 ms | 3.5 |
      | 5 | 0.669 | 0.909 | — | 0.708 | 8,047 | 550 ms | 16.5 ms | 4.0 |
      | 6 (+ p4int) | 0.684 | 0.910 | 0.482 | 0.708 | 8,137 | 590 ms | 16.7 ms | 6.4 |
      | 7 | 0.704 | 0.938 | 0.483 | 0.710 | 8,439 | 717 ms | 17.4 ms | 5.5 |
      | 8 | 0.727 | 0.960 | 0.487 | 0.716 | 8,721 | 617 ms | 17.5 ms | 2.1 |
      | 9 | 0.739 | 0.974 | 0.489 | 0.717 | 8,939 | 633 ms | 18.3 ms | 3.1 |
      | 10 | **0.751** | 0.969 | **0.489** | 0.717 | 9,021 | 670 ms | 18.9 ms | 2.4 |
      | review fixes | 0.745 | 0.957 | **0.491** | 0.716 | 9,524 | 708 ms | 19.9 ms | 2.8 |

      Compile and index are single measurements inside the coverage run (one compile, one
      fresh build). The index build is above the 15 ms target from batch 3 on (the node
      count grew 39%); `sayc_exclusive_index_build_is_bounded` (< 30 ms) holds.
    - Final (batch 10): lints Error 0, `ShadowedBranch` 18 (all ours, the phase-3 rows),
      `DuplicatePath` warnings 23, unconstrained own calls 0. Generated: 2,495 positions
      offer only default passes, 276 overrides in 229 auctions; strict first departures:
      default-pass override 227, their pass not in the trie 7, their call 7, the system
      exhausted 8 (P11: 357 / 60 / 13 / 18). Corpus: all-Exact 0.322 / 0.326 / 0.344 (all /
      eval / subset); system resolution 0.689 / 0.707 / 0.717 (subset eval 0.742); strict
      0.471 / 0.477 / 0.489 (subset eval 0.502), `resolved_at_default_pass` 992 in the
      subset. Agreement: system positions 0.679, natural 0.543. MLE on the tune split: ε
      0.345, δ 0.327, ln L -7378.6. Forward consistency (release, seed `0x5a1c0002`, 10^5):
      0 non-gap violations after every batch; gap-induced 2 / 2 / 3 and `NoCandidate` 28 /
      28 / 47 after batches 8 / 9 / 10 (batch 10's explicit passes leave slam-going hands
      without a row).
    - Why strict [G] stops at 0.751: 227 of the 249 departing auctions leave at a
      default-pass override. The 276 overrides: 119 in auctions they opened with our side
      silent (balancer 70, overcaller 49; e.g. `(2D)-P-(3C)-P-(3D)-P-(P)` 3H,
      `(1H)-P-(2C)-P-(2H)-P-(4H)` 6D with 5 hcp, `(3H)-P-(4NT)` 6C), 78 in our contested
      auctions (opener 43, responder 35; e.g. `1C-(1D)-1H-(1NT)-2H-(P)` 3C with 7 hcp), 38
      uncontested (opener 30, mostly slam moves after partner's game or 3NT, responder 8)
      and 41 after we competed (balancer 16, overcaller 15, advancer 10); by call kind 135
      low suit bids, 63 suit bids at the five level or higher, 50 doubles, 28 notrump bids.
      Most are the natural engine acting where SAYC passes. Writing an explicit pass there
      would be a row that accepts every hand our earlier pass left (class (iii)) or would
      stand in for slam methods the file does not have (Blackwood responses, cue bids); the
      natural engine's appetite is a NaturalParams question (not tuned in this lane).
    - Why strict [C] stops at 0.489: 604 of the subset's 989 calls (batch 7; 992 at batch 10) resolved only by a
      default pass are our passes in the opponents' constructive auctions after our own
      pass; every acting row there contradicts that pass (`ContradictsOwnHistory`), and the
      SAYC answer is the stop pass itself, which the strict count excludes by definition.
      Counting them would need a pass row describing every hand the earlier pass allows,
      i.e. a sink. The natural calls left in the subset are a long tail (the most frequent
      position has 5 calls).
    - Review fixes (after batch 10). The review found that several of the lane's stops
      swallowed hands SAYC does not pass: a strong takeout doubler after advancer's
      cheapest-suit answer, a 12+ advancer, opener after responder's forcing new suit over
      their double, and positions below game after a game force. Fixes:
      - `competing.bml`: after advancer's cheapest-suit answer the doubler passes only with
        the double's minimum range (0--15/16/17 by table) and cue-bids with more; a 12+
        advancer has game, 3NT and cue-bid rows; after advancer's pass over their response
        the strong doubler doubles again (legal also when they bid on, where the cue bid
        was not); a 10+ advance of a double of their raised weak two or of a balancing
        double of a preempt with no stopper and no major cue-bids (the explicit pass rows
        had dropped the table stop, so those hands had no call); the free-advance and
        two-level-overcall advances take their suit as a parameter, so the cue bid is the
        opener's suit; opener's rebid after responder's forcing new suit over a takeout
        double covers every hand and stops only after limited calls.
      - Game forces: the jump shifts (`responses-major.bml`, `continuations-p12.bml`) and
        the 13+ new suits of `rebids.bml` are `GF` (were `F`); nothing stops below game
        after the jump shift or 1m-1X-1NT-2S. Opener never passes 1NT-(2x)-3y (forcing):
        the missing majors and a no-stopper cue bid were added, and only 3NT and the major
        game raise stop. The any-hand sinks under those positions and under
        1H-(1S)-2C-(any) (`competitive-extra.bml`) are gone.
      - Prose: opener's one-level new suit is 12--18 unbalanced (batch 10's 3NT pass is for
        its minimum only), Blackwood is in the system but not written as rows, and no row
        at 1M-1NT-2m compares suit lengths.
      - A new Warning lint, `StopUnderForcing` (06-system.md §9.3 check 9), reports a stop
        pass that is a live candidate (an unshadowed member of some exclusive-index group)
        after partner's forcing call and the opponents' pass, or below game after our game
        force. SAYC has 27, all phase 4 lane D's opener-rebid sinks in `continuations.bml`
        (rebids after one-level and two-over-one responses, 1M-2NT, 1S-2C-2H-3D/3S,
        1H-1S-2C-2D, free bids after their overcall). Those tables do not cover every hand
        yet, so removing the sinks would only turn the hands into implicit passes hidden
        from the audit; they are kept as known warnings and
        `crates/bridge-system/tests/sayc.rs::sayc_stops_under_forcing_calls_are_only_the_known_rebid_sinks`
        pins the set.
      - `xtask coverage` also reports a stop-audited strict [G]: a default pass chosen at a
        position that is not default-pass-only, where the natural engine would not pass, is
        a swallow. Before / after the fixes: strict [G] 0.751 / 0.745, raw [G] 0.969 /
        0.957, stop-audited [G] 0.526 / 0.521, swallows 370 in 307 auctions both times (by
        the natural call: suit 278, notrump 68, double 24; the tops are phase 3's tables:
        the (2S) and (2H) overcaller, 1D-(1H) responder, 1S-(P)-P-(X) opener). The raw and
        strict values fall because the removed stops now go to the natural engine
        (auctions with a natural call 26 -> 38); it does not yet carry a game force forward,
        so responder's own continuation after a jump shift is still a natural pass. Corpus:
        all-Exact 0.320 / 0.323 / 0.341; system resolution 0.688 / 0.708 / 0.716 (subset eval
        0.743); strict 0.472 / 0.479 / 0.491 (subset eval 0.503). MLE: epsilon 0.344, delta
        0.327, ln L -7377.4. Generated positions with no candidate 75 (86 before, 118 before
        the no-candidate fixes). Lints: Error 0, `ShadowedBranch` 18, `DuplicatePath`
        warnings 23, `StopUnderForcing` 27, unconstrained own calls 0; 7,397 rows, 10,220
        trie nodes, 9,524 index nodes. Compile 708 ms and index 19.9 ms in the coverage run
        (loadavg 2.8); `compile_time` best of 3: 690 ms, index 19.8--20.0 ms (loadavg 3.7--3.9).
        Forward consistency (release, seed `0x5a1c0002`, 10^5): 0 non-gap violations, 3
        gap-induced, `NoCandidate` 42, chosen 88,492, implicit pass 11,466 (2.1 s, loadavg
        4.2).
