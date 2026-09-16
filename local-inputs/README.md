# Private images

The private delivery ZIP contains `pokewalker.bin` (49,152 bytes) extracted from
the user's `pw-inputs.zip`, and `eeprom.bin` (65,536 bytes) obtained separately
from the user's File Library. The archive inspected did not contain a 64 KiB
EEPROM image. See `docs/SOURCES.md` for exact hashes and the archive member.

These files are deliberately ignored by Git, are not licensed under MIT, and
must not be published with the code. The CLI only reads them. Guest writes are
exported to a new destination; the original files remain unchanged.

The EEPROM may contain personal device/save information. Derived screenshots and
sound in `private-observations/` are also private. `tools/package.py --private`
explicitly includes both private directories; its default omits them.

Re-import from your own copies into a NEW directory:

```
python3 tools/import_inputs.py --archive /path/to/pw-inputs.zip \
  --eeprom /path/to/eeprom.bin --destination out/imported-inputs
```
