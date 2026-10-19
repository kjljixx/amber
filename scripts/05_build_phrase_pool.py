"""Build assets/phrases.txt, the lyric lines the timer asks Jev to choose from."""

import random
import runpy
from pathlib import Path

from datasets import load_dataset


GENRES = {"rock", "pop"}
POOL_SIZE = 200_000
OUTPUT = Path(__file__).resolve().parent.parent / "assets" / "phrases.txt"


def main():
    candidate_lines = runpy.run_path(
        str(Path(__file__).with_name("03_random_sample.py")),
        run_name="random_sample_module",
    )["candidate_lines"]
    dataset = load_dataset("theelderemo/genius-lyrics-cleaned", split="train")
    indices = [i for i, tag in enumerate(dataset["tag"]) if tag in GENRES]
    print(f"{len(indices):,} of {len(dataset):,} songs are {sorted(GENRES)}")
    random.shuffle(indices)

    pool = {}
    for index in indices:
        lines = candidate_lines(dataset[index]["lyrics"])
        if lines:
            pool.setdefault(random.choice(lines), None)
        if len(pool) == POOL_SIZE:
            break

    OUTPUT.parent.mkdir(exist_ok=True)
    OUTPUT.write_text("\n".join(pool) + "\n")
    print(f"Wrote {len(pool):,} phrases to {OUTPUT}")


if __name__ == "__main__":
    main()
