-- Each device of a shop has its own till (M11/M2): the register it sells
-- on, opens and closes its drawer on. Kept with the device (not shared):
-- another device's copy has its own.
ALTER TABLE device ADD COLUMN register_id TEXT;
