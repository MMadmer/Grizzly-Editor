-- What the Items tab shows of the game's item catalog and of the item check, in Russian
-- (gamedata/configs/item_catalog.ltx; gz-items' validate.rs). Keep the English byte for byte as the
-- catalog and the check say it; a new key of the catalog adds its title and its help here.
--   ctx = "item"      - a title of the catalog: a key's, a group's, a family's, a scope's; a unit
--   ctx = "item_help" - a help sentence of the catalog
--   no ctx            - the check's sentences, with named placeholders
return {
  i18n = 1,
  lang = "ru",
}
