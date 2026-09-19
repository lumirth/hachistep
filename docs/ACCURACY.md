# Hardware accuracy

HachiStep models the Pokéwalker for ordinary use and custom firmware development.
This overview identifies supported behavior and gaps that affect firmware or users.
The topic pages combine our understanding of the hardware with the implementation, its limits and the
reasoning behind it. The [source catalogue](SOURCES.md) identifies the supporting
material and further research leads.

## What can I rely on?

The core implements the target CPU, memory map, clocks, MCU peripherals, connected
sensor, display controller, EEPROM, infrared pins and the effects of power and reset.
Retail and custom firmware use the same mechanisms. Instruction ordering, operations in progress
and device timing remain observable through execution and save states.

Support for a component does not establish every effect of every configuration.
The topic pages describe the scope of each claim. They distinguish hardware evidence
from selected model behavior and link implementation and checks so readers can examine
that distinction themselves.

| Area | Detailed understanding and implementation limits |
| --- | --- |
| CPU | [Execution and exceptions](accuracy/cpu.md), [encodings and arithmetic](accuracy/cpu-arithmetic.md) |
| Memory and pins | [Address decoding, register accesses and GPIO](accuracy/bus-and-gpio.md) |
| Time and power | [Clocks and operating modes](accuracy/clocks.md), [supply, reset and retention](accuracy/power-and-reset.md) |
| Counters | [Timer B1, Timer W and AEC](accuracy/timers.md), [RTC](accuracy/rtc.md), [watchdog](accuracy/watchdog.md) |
| Analog inputs | [ADC and battery sensing](accuracy/adc.md), [comparators](accuracy/comparators.md) |
| Serial interfaces | [SSU](accuracy/ssu.md), [SCI and IrDA](accuracy/sci.md), [IIC2](accuracy/iic2.md) |
| Motion | [BMA150 sampling, filtering and configuration](accuracy/bma150.md), [sensor I²C and board wiring](accuracy/bma150-i2c.md) |
| Display and sound | [NT7508](accuracy/lcd.md), [buttons and buzzer](accuracy/buttons-and-buzzer.md) |
| Persistent memory | [M95512 EEPROM](accuracy/eeprom.md), [internal flash](accuracy/flash.md) |
| Connections | [Infrared transceiver and firmware exchanges](accuracy/infrared.md) |
| Whole machine | [Interactions, restoration and validation](accuracy/interactions.md) |

For frontend integration, start with [API](API.md). For a firmware routine, follow the
hardware it accesses into the relevant topics, including shared clocks, pins and power.
The [design contract](DESIGN.md) describes the intended architecture; these accuracy
pages describe the implemented behavior and the evidence supporting it.

## Known gaps and model limits

These are the main limitations currently identified. Each linked topic gives the
behavior, supporting evidence and specific open questions. Use the table to find the
relevant topic; it does not prescribe an order of work or list every possible defect.

| Area | Gap or model limit | Consequence |
| --- | --- | --- |
| [LCD electrical controls](accuracy/lcd.md#limits-and-open-questions) | Supply, regulator, booster, bias and trim fields have stored values but incomplete electrical effects. | Firmware can change a control without the corresponding change in displayed drive or contrast. |
| [Sensor calibration](accuracy/bma150.md#offset-calibration-and-gain-fields) | Gain, temperature trim and some protected fields are stored, but their effect on sensor readings is not established. | Offset calibration works, but other calibration writes cannot yet predict their physical response. |
| [Optical transceiver](accuracy/infrared.md#optical-transceiver) | The model handles signal levels and shutdown. It does not yet reshape received pulses, delay reception after transmission or cut off prolonged transmission. The component is unidentified. | Two emulated walkers communicating does not establish agreement with another kind of peer or with weak or distorted optical signals. |
| [Battery sensing](accuracy/adc.md#selected-nominal-circuit-and-defaults) | Firmware constrains the response, but the circuit and its nominal voltage drop remain inferred. | ADC results for supplied pin voltages have a stronger basis than predicted battery-warning voltage. |
| [Power and retention](accuracy/power-and-reset.md) | Reset capacitance, oscillator startup, loss of stored state at low voltage and some board connections use selected approximations. | Short supply dips and startup near electrical limits need care when drawing conclusions about a physical unit. |
| [EEPROM](accuracy/eeprom.md#a-compact-default-for-interrupted-programming) and [flash](accuracy/flash.md#concrete-nominal-partial-progress-model) | The model tracks erase/program progress. The time or pulse exposure needed to change individual bits is approximate. | Recovery experiments exercise interrupted operations without predicting the exact damaged bits of an individual device. |
| [Live changes and interactions](accuracy/interactions.md#review-boundaries) | Some clock, serial, capture and reset races have documented rules; others use local inferences or have narrower checks. | A correct isolated operation does not establish every combination with another active device. |
| [Physical presentation](accuracy/buttons-and-buzzer.md) | Digital drive is available; glass response and piezo/enclosure acoustics are not calibrated. | Frontends can present the device, while a physical appearance or acoustic model remains separate work. |

Known omissions deserve implementation work even when retail firmware rarely exercises
them. A plausible approximation can remain useful while we refine its parameters.
Choose work by its consequences for ordinary use, custom firmware, performance and
maintainability; the length of a topic or its test count does not set its priority.

The selected analog values describe a representative device. Differences between
individual units become implementation work when their effects matter to the supported
use, such as a firmware decision near a voltage or motion threshold.

## Understanding the evidence

Hardware knowledge and implementation correctness are separate questions. A documented
mechanism can be missing in code. A mechanism absent from a datasheet can still have a
strong implementation basis in firmware, vendor code and circuit reasoning.

| What we know | How the topic should describe it |
| --- | --- |
| Hardware behavior is understood and implemented with supporting checks. | State the behavior, its scope, the source reasoning and what the checks observe. |
| Hardware behavior is understood but its effect is missing or wrong. | Identify the implementation gap and its consequence. Further discovery is not a prerequisite to fixing it. |
| The implementation follows a justified inference. | Explain the inference and the particular assumptions that affect the result. |
| A hardware question remains unresolved. | State what the sources establish, what is still unclear, and useful avenues for resolving it. |
| An area has received limited examination. | Describe the boundary of that examination without declaring the area correct or defective. |

These distinctions show what we understand and where work remains. We cannot list
problems we have not discovered, but we can explain which behavior we have examined,
especially where components interact. Running retail firmware tells us less about
unused hardware modes than a diagnostic written to exercise those modes. Exact save
state restoration shows that emulation resumes consistently; agreement with the
physical device needs its own evidence.

Useful knowledge can also exist without reaching the implementation or these pages.
A cited application note may contain an overlooked timing diagram; a firmware routine
may contradict a register interpretation; a code comment may hold the only explanation
of a model choice. Follow those leads into the topic that owns the behavior. A source
being listed does not mean every relevant passage has been understood or applied.

Strong inference can justify implementation. Physical measurement can strengthen or
correct it, but new measurements are not a prerequisite for supported behavior.
An inferred mechanism can be considered supported when it explains the applicable
sources and observed firmware behavior coherently. Keep its basis beside the mechanism;
revisit it when evidence conflicts or another plausible interpretation would change
behavior that matters to firmware or users.
Inference alone is not an implementation gap.

Search applicable manuals, amendments, vendor examples, board evidence and the matching
firmware before describing a mechanism as undocumented. Preserve conflicts and the
reason for resolving them in the detailed topic.

## Keeping this useful

When behavior or its supporting evidence changes, update the affected topic and any
consequential limitation summarized here. A resolved gap becomes a description of the
supported behavior. Keep interpretation beside the mechanism, source applicability in
[SOURCES](SOURCES.md), and check commands and evidence limits in [TESTING](TESTING.md).
Issues hold investigations and commits hold change history. Individual run reports
remain with the run.

Split a topic when its mechanisms need independent explanation or become difficult to
navigate. Consolidate overlapping accounts as part of the change, preserving useful
reasoning and updating links. The topic pages own hardware accuracy; the remaining
`research/` notes compare emulator architecture and interface designs.
