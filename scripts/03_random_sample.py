
import random

from datasets import load_dataset


def candidate_lines(lyrics):
    return [
        line.strip()
        for line in lyrics.splitlines()
        if line.strip()
        and not line.strip().startswith("[")
        and 3 <= len(line.split()) <= 12
    ]


def select_samples():
    dataset = load_dataset(
        "theelderemo/genius-lyrics-cleaned",
        split="train",
    )

    samples = []
    for index in random.sample(range(len(dataset)), 1000):
        song = dataset[index]
        if song["tag"] not in {"rock", "pop"}:
            continue

        lines = candidate_lines(song["lyrics"])
        if lines:
            samples.append(
                {
                    "artist": song["artist"],
                    "title": song["title"],
                    "tag": song["tag"],
                    "lyric": random.choice(lines),
                }
            )
    return samples


def main():
    samples = select_samples()
    print(f"Selected {len(samples)} samples")
    for sample in samples:
        print(
            f"{sample['artist']} - {sample['title']} ({sample['tag']})"
        )
        print(f"  {sample['lyric']}\n")


if __name__ == "__main__":
    main()
