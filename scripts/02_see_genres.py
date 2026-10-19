from datasets import load_dataset
from collections import Counter

dataset = load_dataset(
  "theelderemo/genius-lyrics-cleaned",
  split="train"
)

genres = Counter(dataset["tag"])

for genre, count in genres.most_common():
  print(f"{genre}: {count:,} ({count / len(dataset):.1%})")