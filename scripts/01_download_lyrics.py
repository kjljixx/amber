
from datasets import load_dataset

dataset = load_dataset(
  "theelderemo/genius-lyrics-cleaned",
  split="train"
)

print(f"Loaded {len(dataset):,} songs")
dataset.to_csv("lyrics.csv")
print("Saved complete dataset to lyrics.csv")
