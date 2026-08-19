-- What the relay remembers about a card it cannot address.
--
-- Additive only, same as 0002, 0003 and 0004 and for the same reason: the previous
-- worker is still serving requests while a deploy rolls out, and an App Store
-- build from months ago still calls these routes. Both columns are nullable and
-- a row that predates them behaves exactly as it did.
--
-- Both exist because of one gap: the relay can START a card on a phone whose app
-- is not running, and only the app can learn the update token that addresses it.
-- Between those two moments `live_activities` holds a row with the
-- `TOKEN_UNKNOWN` sentinel — a card the relay knows exists and cannot move — and
-- everything below is about not making that card lie.

-- What the card the relay started blind is showing.
--
-- 'working' or 'blocked': the status it was push-started with. NULL once the app
-- files a real update token, because from then on the card is addressable and
-- the relay changes it in place instead of guessing.
--
-- It is what lets a `blocked` push replace a card stuck on "Working". Before
-- this, a working push claimed the row, the blocked push that followed found a
-- row it could not address and returned — leaving the lock screen reading
-- "Working" for an agent whose banner said it needed you, on the exact scenario
-- this product exists for. Remembering the tier is what keeps that replacement
-- to ONE card: a second blocked push finds 'blocked' here and starts nothing.
ALTER TABLE live_activities ADD COLUMN blind_status TEXT;

-- When the person swiped the card away, in Unix milliseconds. NULL is "still up".
--
-- A dismissal used to delete the row, and the daemon pushes a working card every
-- ten seconds — so the next push found nothing running and started the card
-- again, for the length of the run. A card that comes back within ten seconds of
-- being dismissed is a card that cannot be dismissed at all.
--
-- The row therefore outlives the card, remembering the refusal until something
-- changes that the person has not already answered: a `blocked` push is new news
-- and raises a fresh card, and a `done` push deletes the row with the run.
ALTER TABLE live_activities ADD COLUMN dismissed_at INTEGER;
