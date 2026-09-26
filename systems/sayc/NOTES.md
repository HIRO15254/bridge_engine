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
