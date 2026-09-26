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
    a distinct, lower `6--10 hcp` band, making it a genuine preemptive jump
    (extra length *and* a capped strength), not merely "the same hand, one
    card longer."

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
    (22--24), 3NT 3--8, 4NT 9--10, 6NT 11+, and 4C only with 13+. Stayman
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
