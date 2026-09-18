# BMA150 continuous response

Implemented 2026-09-18 after the [completion review](bma150-completion-review.md).
Bosch specifies a second-order 1500 Hz analog filter before the ADC and its
separate digital moving average (§3.1.3 p.12, §8.1 p.51 of
[BST-BMA150-DS000-06](https://media.digikey.com/pdf/Data%20Sheets/Bosch/BMA150.pdf)).
The physical stage now retains a pulse wholly between conversions. Its
maximally flat damping is a selected realization of those two poles; the
datasheet does not establish the damping numerically.

## State and evolution

Each axis retains output `y`, normalized derivative `d = y'/k`, and the physical
instant those values describe, where `k = sqrt(2)*pi*1500` per second. For
constant input `u` over `dt`, with `r+i*s = exp((-1+i)*k*dt)`, advance together:

```text
y_next = u + (r+s)*(y-u) + s*d
d_next = (r-s)*d - 2*s*(y-u)
```

The input trajectory remains piecewise constant. Changes settle the previous
input before replacing it; unchanged values do not introduce rounding points.
Actual axis apertures and power/sleep transitions are the other evolution
boundaries. Host horizons, reads, inspection and save/load do not evolve the
physical state. This adds no scheduled events or allocations.

Signals use signed Q24 micro-g and coefficients use Q48, with signed nearest
rounding, ties away from zero, and widened `i128` products. The final analog
value rounds to the physical input's one-micro-g resolution before the existing
offset/range and signed ADC truncation. This rounding is explicit, separate
from retained sub-micro-g filter state. It avoids exposing infinitesimal
startup residuals as persistent whole-code errors at an exact ADC threshold.

`k` is `28_623_055_379_020` in Q32. A fixed Taylor polynomial of the reduced
complex exponential followed by squaring evaluates arbitrary intervals; no
host floating-point math enters execution. The ordinary 1/3000-second interval
has compile-time coefficients (its two adjacent 64.64 representations select
the same Q48 values). Exact equilibrium requires no multiplication. At eight
milliseconds the entire allowed residual is below one stored signal bit, so
the rounded coefficients are zero before any large timestamp multiplication.

The numerical probe in `out/analog_coefficients.rs` compared 82,475 separate
intervals through 8 ms against floating-point exponent/trigonometric functions;
maximum coefficient error was 8.596e-14. Production behavior is checked against
independent continuous step values and sinusoidal gains at 300, 1500 and 6000 Hz,
including the zero-order hold's attenuation. Those calculations validate the
selected equations, not a measured Bosch impulse response.

Warm sleep and retained undervoltage freeze the state at entry. Changed physical
input remains available, and wake rebases the integration instant and allows
settling through the entire existing readiness interval. Cold power and soft
reset start at zero. These charge-retention/reset choices follow the review;
they add neither a hidden sleep acquisition path nor an invented leakage rate.

Native state includes all three axis records (96 bytes), independent of ADC
history. Validation checks chronology, including the supply-loss boundary,
input limits, and the invariant radius `y*y + (y+d)*(y+d) <= (4U)^2` for
`U=1e9 micro-g`. Individual bounds precede squaring. The filter is not clamped
to the ADC range; physical overshoot is preserved until conversion.

## Behavioral validation

The unit pulse is +1 g on X from 3200 to 3250 microseconds, between apertures at
3166 2/3 and 3500. Independent step subtraction predicts 103.2349815 mg at the
following aperture, or code 26 at +/-2 g. The core produces that value.
Generated run partitions include native capture during the pulse and compare
all subsequent events and complete causal state. The allocation gate now
includes a short pulse as well as constant conditions and nonvolatile work.

`hachiware` independently polls the unshadowed X MSB and latches detection,
without fixing an exact amplitude. The pulse guest fails against the preceding
direct-sampling executable (`out/analog-pulse-before.json`); the zero-input
control passes. Both pass with this filter. The existing image/shadow diagnostic
now waits for analog settling and ignores the unrelated, asynchronously updated
freshness bit when comparing its final data pair. Its held-MSB assertion remains.

All 125 guests pass in `out/analog-conformance3.json`. Full gate and retail
receipts are `out/analog-check2` and `out/analog-retail-reviewed`.
The 24 sensor tests also pass on Rust 1.95.0 (`out/analog-msrv.log`).

## Reviewed retail consequence

The home, menu and idle workloads retain all their reviewed observations. The
walking timeline is a 10 ms piecewise-constant force trajectory; its acceleration
transitions now settle before ADC publication. Compared with
`out/host-completion-retail/walking`, the 61-second run changes twelve RAM bytes
in F7E6..F850, retires 16,779,565 instructions instead of 16,779,567, and performs
26,770,715 reads instead of 26,770,707. `RuntimeState`, final CPU registers,
interrupts, product/serial totals, display and persistent exports are unchanged.
The matching `pw` [workspace layout](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/include/globals.h)
and [motion processing](https://github.com/lumirth/pw/blob/6dc7bc09950078fa3fe0dffa4dae34e9549a99da/src/application/pw_pedometer.c)
explain why altered sensor samples can change working RAM and executed arithmetic
without changing the final display or persistent progress.

Only those three walking expectations were revised. The initial regression
differences remain in `out/analog-retail`; the verifier did not update them.
No firmware image or EEPROM input was changed.
