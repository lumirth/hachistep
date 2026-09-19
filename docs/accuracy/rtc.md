# RTC

[Accuracy overview](../ACCURACY.md) · [Source catalogue](../SOURCES.md)

## Supported behavior

Calendar counting, the alternate free-running counter, quarter/half-second events,
interrupt timing selection, clock output and reset-domain distinctions are implemented.
RTC data and controls survive MCU RES/watchdog reset where specified. Software RTC
reset has its own effects. A pending calendar update and the busy interval survive
suspension and save/restore.

## Limits and open questions

The manual and same-target RTC application note specify approximately 62.5 ms between
busy assertion and the data update. HachiStep uses 512 watch/4 ticks, placed at ticks
7680 through 8192 of its second. The initial placement is inferred. Live writes during
busy interact with a latched prospective update; malformed BCD follows digit counters.
These choices matter for firmware intentionally racing an update or using invalid
calendar fields. Ordinary stable reads and calendar rollover have direct support.

## Sources and applicability

The H8/38606 target addition leaves these peripherals unchanged. The applicable baseline
is REJ09B0152-0300 rev. 3.00. Page references are printed manual pages; PDF pages are 34
higher. ([Target applicability][target], [manual][manual])

## RTC: retained state, busy interval and raw counters

RTCFLG, time data and RTCCR1/2 survive RES/watchdog reset. RTCCSR resets to `08`, as
specified separately in section 20.3. Software `RTCCR1.RST=1` instead resets RTC
registers/control circuits except RTCCSR and RST itself; software must clear RST. Cold
RTC contents are not specified; zero remains a reasonable deterministic construction
value, distinct from these reset rules. ([Manual pp. 191, 196, 199, 380][rtc-reset])

The stable read contract is specific: BSY becomes one, approximately 62.5 ms later data
registers update and BSY clears. INT selects periodic interrupts during busy or
immediately afterward. The model uses a 512-tick `φW/4` busy interval followed by a
separate commit. Its placement at ticks 7680–8192 is an inference; neither the manual
nor the same-target RTC application note fixes first-BSY phase after RUN. Retain this
local phase choice; there is no basis to invent a staggered per-register update
sequence. ([Manual §11.4.3][rtc-busy], [RTC Operation §3.2–3.3, pp. 8–9][rtc-app])

Keep quarter/half-second divider events distinct from calendar update. Set RTCFLG flags
only for enabled RTCCR2 sources; clearing is write-zero, with no prior-read
qualification. Calendar mode uses `φW/4`; the other documented sources make RSECDR a
full 8-bit binary free-running counter. In calendar mode bit 7 reads BSY, not a raw
time-data bit. Stop/gating preserves phase; RTC RST clears it. Watch calendar counting
continues through watch/subactive/subsleep but halts in standby. ([Manual §§11.3, 11.5;
table 5.3][rtc])

RTCCSR also selects TMOW: upper-field values `000/010/100/110` select `φ/4,/8,/16,/32`,
and `xx1` selects φW. Route this through PMR1's existing pin mux from the actual divider
signal; changing clock output must not reset time counting. ([Manual §11.3.7, p.
193][rtc])

Live configuration follows these rules:

- Time write while RUN: accept the documented writable field bits
  (`7F,7F,3F,07`) without resetting the divider. The prescribed software
  sequence stops and resets before setting time; it does not describe a hardware
  write lock. As a compact race model, latch the prospective update at busy
  entry; writes change visible storage, and an already-started update commits
  its latched result afterward. Stopping freezes that update; RST cancels it.
  These race outcomes are implementation inferences.
- Malformed BCD: preserve raw field widths and use digit-counter terminal
  tests. For seconds/minutes, units `9` clears and carries; otherwise increment
  the four-bit units field. On a units carry, tens `5` clears and carries;
  otherwise increment its three-bit field. For hours, `11`/`23` is the selected
  terminal, with ordinary BCD digit carry otherwise. Day `6` clears and carries;
  other values increment modulo eight. Thus invalid digits wrap by their
  physical widths, not by calendar normalization, and never cause a Rust
  overflow or an emulator error. PM toggles only at the 12-hour terminal; day
  advances when PM changes from one to zero; the weekly condition checks the
  resulting day value for zero. This is a small comparator/counter
  inference, consistent with the described digit structure.
- RTCCSR codes 9–15: retain the written register and decode bit 3 as watch
  calendar selection. This has stronger evidence than a guessed clock: the
  same-target 2005 application note explicitly lists `1xxx` as RTC operation,
  whereas rev. 3 narrows permitted programming to `1000`. Treat the alias as an
  inference, not a reason to advertise those settings as supported hardware
  programming practice. ([ADC application note p. 12][adc-app])

TMOW and CLKOUT are routed through the P10 pin mux, independently of RTC RUN. The
scheduler visits their actual high/low transitions only while that alternate output is
selected. This matters because P10 is the board's LCD select. CLKOUT uses φOSC, /2 or
/4; selector 111 releases the output as the local rule. Standby releases the output,
while stopped sources otherwise retain the last level. ([Manual §8.1.4 and pin table,
pp. 122–123][gpio-clock])

## Firmware use

[`pw` RTC setup and reads](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_rtc.c#L164-L206)
reset the RTC, select interrupts after busy, and take two complete busy-qualified
snapshots. This corroborates the stable-read mechanism while leaving the initial busy
phase and writes during busy as the specific inferences described above.

## Implementation and checks

The [RTC implementation](../../crates/hs-core/src/mcu/rtc.rs) owns time, busy state
and the prospective update. Hachiware's
[RTC cases](https://github.com/lumirth/hachiware/blob/main/cases/rtc.py) cover reset
retention, busy timing, calendar behavior and clock output. Cases that depend on initial
phase or malformed BCD exercise the stated inference. The
[day-rollover workload](../TESTING.md#private-firmware-regression-versus-smoke) adds a
firmware consequence and restoration during busy. Its software baseline does not
independently establish the initial busy phase.

[gpio-clock]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=156
[manual]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual
[target]: https://www.renesas.com/en/document/tcu/addition-h838606-group#page=2
[rtc]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=221
[rtc-reset]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=414
[rtc-busy]: https://www.renesas.com/en/document/mah/h838602r-group-hardware-manual#page=231
[rtc-app]: https://www.renesas.com/en/document/apn/h838602r-group-rtc-operation#page=10
[adc-app]: https://www.renesas.com/en/document/apn/h838602r-group-application-note-ad-conversion-using-subclock#page=14
