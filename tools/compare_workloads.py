#!/usr/bin/env python3
"""Compare supplied native workload adapters with behavioral preflight and work counts.

The default adapter accepts MODE ROM QUANTUM_US LIMIT_US 1 and prints one JSON
object containing ns, retired and the endpoint fields below. Timing must exclude
construction and final hashing; adapters define their output consumer. A separate
profile-work executable can emit the same endpoint plus a nested `work` object.

Mark our workload_probe adapters with --artifacts NAME. Their preflight receives
--artifact-dir and exports a native state plus complete product-event history.
Legacy adapters establish selected endpoint observations only; event_hash is lossy.
Equivalence groups make behavioral differences explicit when comparing cores.
"""

from __future__ import annotations

import argparse
import json
import math
import platform
import random
import statistics
import subprocess
import time
from pathlib import Path

try:
    import resource
except ImportError:
    resource = None

from _support import create_directory, digest, environment, source_identity, write_json

ENDPOINT_KEYS = (
    "horizon_us", "calls", "retired", "pc", "ccr", "registers", "ram_hash",
    "lcd_hash", "eeprom_hash", "ssu_counts", "events", "event_hash", "sleeping",
    "completed",
)
BYTE_KEYS = {"ssu_tx_bytes": "ssu_counts.0", "ssu_rx_bytes": "ssu_counts.1"}


def field(value: dict, path: str):
    for part in path.split("."):
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def assignments(items: list[str]) -> dict[str, str]:
    result = {}
    for item in items:
        name, separator, value = item.partition("=")
        if not separator or not name or not value or name in result:
            raise ValueError(f"expected distinct NAME=VALUE assignments: {item}")
        result[name] = value
    return result


def counts(value: dict, prefix: str = "") -> dict[str, int]:
    result = {}
    for key, count in value.items():
        name = f"{prefix}{key}"
        if isinstance(count, dict):
            result.update(counts(count, name + "."))
        elif type(count) is int and count >= 0:
            result[name] = count
        else:
            raise ValueError(f"work counter {name} must be a nonnegative integer")
    if not result:
        raise ValueError("work accounting contains no counters")
    return result


def normalize(totals: dict[str, int], report: dict, byte_keys: dict) -> dict:
    denominators = {"retired": report["retired"]}
    denominators.update({name: field(report, key) for name, key in byte_keys.items()})
    if any(type(n) is not int or n < 0 for n in denominators.values()):
        raise ValueError("retirement/byte counts must be nonnegative integers")
    counts_are_cumulative = report.get("cumulative_counts") is True or report.get("resets", 0) == 0
    return {
        "totals": totals,
        "denominators": denominators,
        "per": {
            name: {
                key: count / n if n and counts_are_cumulative else None
                for key, count in totals.items()
            }
            for name, n in denominators.items()
        },
        "count_scope": (
            "adapter declares cumulative counts" if report.get("cumulative_counts") is True
            else "reset-free fixed-condition board run" if counts_are_cumulative
            else "counts may restart at reset; ratios unavailable"
        ),
    }


def schedule(names: list[str], repeats: int, seed: int) -> list[tuple[int, str]]:
    # Each paired block has reversed halves; shuffle the next block to vary order
    # while giving every variant equal observations at each end of a block.
    rng = random.Random(seed)
    result = []
    for repeat in range(repeats):
        order = names.copy()
        rng.shuffle(order)
        result.extend((repeat, name) for name in order + order[::-1])
    return result


def endpoint(report: dict, keys: list[str] | tuple[str, ...]) -> dict:
    return {key: field(report, key) for key in keys}


def history(report: dict, directory: Path) -> dict:
    value = report["history"]
    if value.get("complete") is not True or type(value.get("records")) is not int or value["records"] < 0:
        raise ValueError("history must declare complete=true and a nonnegative record count")
    path = Path(value["path"])
    path = path if path.is_absolute() else directory / path
    with path.open("rb") as stream:
        records = sum(1 for _ in stream)
    if records != value["records"] or records != report["events"]:
        raise ValueError("history record count differs from complete exported events")
    state = directory / "state.bin"
    return {"sha256": digest(path), "records": value["records"], "state_sha256": digest(state)}


def observe(variant: dict, case: dict, phase: str, directory: Path, timeout: float) -> dict:
    directory.mkdir()
    adapter = variant["work_adapter"] if phase == "work" else variant["adapter"]
    command = [adapter, case["mode"], case["rom"], str(case["quantum_us"]), str(case["limit_us"]), "1"]
    if phase == "preflight" and variant.get("artifacts"):
        command += ["--artifact-dir", str(directory)]
    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    start = time.perf_counter()
    process = subprocess.run(
        command, capture_output=True, text=True, env=environment(),
        timeout=timeout, check=False,
    )
    wall = time.perf_counter() - start
    usage_after = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    cpu = (
        usage_after.ru_utime + usage_after.ru_stime - usage_before.ru_utime - usage_before.ru_stime
        if usage_before is not None and usage_after is not None else None
    )
    (directory / "stdout.txt").write_text(process.stdout)
    (directory / "stderr.txt").write_text(process.stderr)
    receipt = {"command": command, "returncode": process.returncode, "process_wall_seconds": wall, "process_cpu_seconds": cpu}
    write_json(directory / "invocation.json", receipt)
    if process.returncode:
        raise RuntimeError(f"adapter failed: {directory}")
    report = json.loads(process.stdout)
    if not isinstance(report, dict):
        raise ValueError("adapter reply must be a JSON object")
    if phase != "work" and "work" in report:
        raise ValueError("instrumented reply used as an uninstrumented observation")
    return {"report": report, **receipt}


def summarize(samples: list[dict], byte_keys: dict) -> dict:
    times = [sample["report"]["ns"] for sample in samples]
    if any(type(n) not in (int, float) or not math.isfinite(n) or n <= 0 for n in times):
        raise ValueError("adapter ns must be positive finite simulation wall time")
    metrics = {
        "retired_per_second": [
            sample["report"]["retired"] * 1e9 / n
            for sample, n in zip(samples, times)
        ]
    }
    metrics.update({
        f"{name}_per_second": [
            field(sample["report"], key) * 1e9 / n
            for sample, n in zip(samples, times)
        ]
        for name, key in byte_keys.items()
    })
    return {
        "samples": len(times), "median_ns": statistics.median(times),
        "min_ns": min(times), "max_ns": max(times),
        "throughput_medians": {key: statistics.median(values) for key, values in metrics.items()},
    }


def paired_blocks(rows: dict[str, list[dict]], baseline: str, block_runs: int = 1) -> list[dict]:
    blocks = []
    repeats = sorted({sample["repeat"] for sample in rows[baseline]})
    for repeat in repeats:
        means = {}
        for name, samples in rows.items():
            block = [sample["report"]["ns"] for sample in samples if sample["repeat"] == repeat]
            if len(block) != 2 * block_runs:
                raise ValueError("paired block must contain two samples per variant per block run")
            means[name] = statistics.mean(block)
        blocks.append({
            "repeat": repeat,
            "mean_ns": means,
            "time_ratio_to_baseline": {name: value / means[baseline] for name, value in means.items()},
        })
    return blocks


def compare_case(case: dict, variants: dict, directory: Path, repeats: int, seed: int, timeout: float, block_runs: int = 1) -> dict:
    directory.mkdir()
    rom_hash = digest(Path(case["rom"]))
    keys, byte_keys = ENDPOINT_KEYS, BYTE_KEYS
    preflights, expected, histories, groups, history_groups = {}, {}, {}, {}, {}
    # Complete all behavioral checks before entering any timed block.
    for name, variant in variants.items():
        result = observe(variant, case, "preflight", directory / f"preflight-{name}", timeout)
        report = result["report"]
        if case["mode"] == "job" and report.get("completed") is not True:
            raise ValueError(f"{name} did not complete {case['name']}")
        expected[name] = endpoint(report, keys)
        preflights[name] = result
        if variant.get("artifacts"):
            histories[name] = history(report, directory / f"preflight-{name}")
        group = variant.get("group", "all")
        if group in groups:
            reference = groups[group]
            if expected[name] != expected[reference]:
                write_json(directory / "preflight.json", {"observations": preflights, "histories": histories, "equivalent": False})
                raise ValueError(f"behavioral preflight differs within group {group}: {reference}, {name}")
        else:
            groups[group] = name
        if name in histories:
            if group in history_groups and histories[name] != histories[history_groups[group]]:
                write_json(directory / "preflight.json", {"observations": preflights, "histories": histories, "equivalent": False})
                raise ValueError(f"native state/history preflight differs within group {group}")
            history_groups.setdefault(group, name)
    work = {}
    for name, variant in variants.items():
        if variant.get("work_adapter"):
            result = observe(variant, case, "work", directory / f"work-{name}", timeout)
            if endpoint(result["report"], keys) != expected[name]:
                raise ValueError(f"instrumented endpoint differs: {name}")
            work[name] = {**result, "accounting": normalize(counts(result["report"]["work"]), result["report"], byte_keys)}
    references = list(groups.values())
    differences = {
        name: {key: {"reference": expected[references[0]][key], "value": value} for key, value in expected[name].items() if value != expected[references[0]][key]}
        for name in references[1:]
    }
    preflight = {
        "scope": "selected endpoints for every adapter; native state and complete Event Debug history for artifact-capable adapters within each group",
        "groups": {name: variant.get("group", "all") for name, variant in variants.items()},
        "observations": preflights,
        "histories": histories,
        "artifact_variants": list(histories),
        "cross_group_differences": differences,
        "equivalent_within_groups": True,
    }
    write_json(directory / "preflight.json", preflight)
    rows = {name: [] for name in variants}
    index = 0
    for slot, (repeat, name) in enumerate(schedule(list(variants), repeats, seed)):
        for block_run in range(block_runs):
            result = observe(variants[name], case, "timing", directory / f"timing-{index:03}-{name}", timeout)
            if endpoint(result["report"], keys) != expected[name]:
                raise ValueError(f"timed endpoint differs from its preflight: {name}")
            rows[name].append({"repeat": repeat, "order_index": index, "slot": slot, "block_run": block_run, **result})
            index += 1
    summaries = {name: summarize(samples, byte_keys) for name, samples in rows.items()}
    reference = next(iter(variants))
    blocks = paired_blocks(rows, reference, block_runs)
    for name, summary in summaries.items():
        summary["time_ratio_to_baseline"] = summary["median_ns"] / summaries[reference]["median_ns"]
        ratios = [block["time_ratio_to_baseline"][name] for block in blocks]
        summary["paired_time_ratio"] = {
            "median": statistics.median(ratios), "min": min(ratios), "max": max(ratios),
        }
    if digest(Path(case["rom"])) != rom_hash:
        raise RuntimeError("ROM changed during comparison")
    return {
        "case": case, "rom_sha256": rom_hash, "preflight": preflight,
        "work": work, "raw_samples": rows, "paired_blocks": blocks,
        "summary": summaries, "baseline": reference,
    }


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--variant", action="append", default=[], help="NAME=uninstrumented native adapter")
    parser.add_argument("--work-variant", action="append", default=[], help="NAME=separate profile-work adapter")
    parser.add_argument("--group", action="append", default=[], help="NAME=equivalence group; default all")
    parser.add_argument("--artifacts", action="append", default=[], help="NAME supports workload_probe native state/history preflight")
    parser.add_argument("--build-receipt", action="append", default=[], help="NAME=source/compiler/profile build receipt file")
    parser.add_argument("--rom-dir", type=Path, required=True, help="directory of supplied board_*.bin workload images")
    parser.add_argument("--case", action="append", default=[], help="select a ROM stem")
    parser.add_argument("--quantum-us", type=int, action="append", help="repeat to select caller horizons; default 1000")
    parser.add_argument("--mode", choices=("job", "time"), help="override job completion or fixed-duration execution")
    parser.add_argument("--limit-us", type=int, help="override maximum simulated duration")
    parser.add_argument("--repeats", type=int, default=3, help="paired blocks; two samples per variant per block")
    parser.add_argument("--block-runs", type=int, default=1, help="fresh executions at each schedule position; every ns/process-wall sample is retained")
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.repeats < 2 or args.block_runs < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("repeats must be >=2, block-runs >=1 and timeout positive")
    variants = {}
    for name, path in assignments(args.variant).items():
        variants.setdefault(name, {})["adapter"] = str(Path(path).resolve())
    for option, key in ((args.work_variant, "work_adapter"), (args.group, "group"), (args.build_receipt, "build_receipt")):
        for name, value in assignments(option).items():
            if name not in variants:
                parser.error(f"unknown variant: {name}")
            variants[name][key] = str(Path(value).resolve()) if key != "group" else value
    for name in args.artifacts:
        if name not in variants:
            parser.error(f"unknown artifact variant: {name}")
        variants[name]["artifacts"] = True
    identities = {}
    for name, variant in variants.items():
        if not name.replace("-", "").replace("_", "").isalnum():
            raise ValueError("variant names must contain letters, numbers, hyphens or underscores")
        identities[name] = {"group": variant.get("group", "all")}
        for key in ("adapter", "work_adapter", "build_receipt"):
            if variant.get(key):
                path = Path(variant[key]).resolve()
                variant[key] = str(path)
                identities[name][key] = {"path": str(path), "sha256": digest(path)}
    if not variants:
        parser.error("supply at least one --variant")
    cases = [
        {
            "name": path.stem,
            "rom": str(path.resolve()),
            "mode": "time" if "idle" in path.stem else "job",
            "limit_us": 1_000_000 if "idle" in path.stem else 1_000_000_000,
        }
        for path in sorted(args.rom_dir.glob("board_*.bin"))
    ]
    if args.case:
        cases = [case for case in cases if case["name"] in args.case]
        missing = set(args.case) - {case["name"] for case in cases}
        if missing:
            parser.error(f"unknown cases: {sorted(missing)}")
    matrix = []
    for case in cases:
        for quantum in args.quantum_us or [1000]:
            value = {
                **case,
                "mode": args.mode or case["mode"],
                "quantum_us": quantum,
                "limit_us": args.limit_us if args.limit_us is not None else case["limit_us"],
            }
            if min(value["quantum_us"], value["limit_us"]) < 1 or value["mode"] not in ("job", "time"):
                parser.error("case mode must be job/time and quantum/limit positive")
            matrix.append(value)
    if not matrix:
        parser.error("no board_*.bin workloads selected")
    out = create_directory(args.out)
    for name, variant in variants.items():
        if variant.get("build_receipt"):
            receipt = out / f"build-{name}.txt"
            receipt.write_bytes(Path(variant["build_receipt"]).read_bytes())
            identities[name]["build_receipt"]["retained_path"] = str(receipt)
    source = source_identity()
    report = {
        "schema": 1, "platform": platform.platform(), "tool_source": source,
        "variants": identities, "seed": args.seed, "repeats": args.repeats, "block_runs": args.block_runs,
        "results": [],
        "limitations": [
            "Executable hashes identify supplied bytes; build receipts are separately supplied evidence, not verified source provenance.",
            "Work accounting uses separate instrumented runs; their wall time is excluded from timing summaries.",
            "Cross-group endpoint differences remain visible; this comparison does not establish hardware fidelity.",
            "Legacy normalization assumes the board adapter's fixed conditions and no external inputs; a reset counter alone does not prove power continuity.",
            "Mirrored paired blocks reduce ordering bias; host contention and drift can still invalidate a speed claim. Inspect raw samples and paired ratio spread.",
        ],
    }
    try:
        for index, case in enumerate(matrix):
            result = compare_case(
                case, variants, out / f"{index:03}-{case['name']}-{case['quantum_us']}us",
                args.repeats, args.seed + index, args.timeout, args.block_runs,
            )
            report["results"].append(result)
            print(case["name"], case["quantum_us"], {name: round(value["median_ns"] / 1e6, 3) for name, value in result["summary"].items()}, flush=True)
    except Exception as error:
        report["failure"] = str(error)
        raise
    finally:
        report["tool_source_after"] = source_identity()
        report["tool_source_unchanged"] = source.get("tree_sha256") == report["tool_source_after"].get("tree_sha256") if source.get("tree_sha256") else None
        report["adapter_bytes_unchanged"] = all(
            digest(Path(identity[key]["path"])) == identity[key]["sha256"]
            for identity in identities.values()
            for key in ("adapter", "work_adapter") if key in identity
        )
        write_json(out / "summary.json", report)
    if not report["adapter_bytes_unchanged"]:
        raise RuntimeError("adapter changed during comparison; summary retained")
    if report["tool_source_unchanged"] is False:
        raise RuntimeError("checkout changed during comparison; summary retained")


if __name__ == "__main__":
    main()
