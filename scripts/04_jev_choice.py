"""Ask Jev to choose among lyric samples selected by 03_random_sample.py."""

import json
import os
import runpy
from pathlib import Path
from urllib.request import Request, urlopen


ENDPOINT = "https://openrouter.ai/api/alpha/decisions"
MODEL = "typesafe/jev-1.13"

# Replace these placeholders when the choice question and context are settled.
STATE = {
}
QUESTION_INSTRUCTIONS = ("Choose the best potential atmospheric phrase."
                        "Assess whether it is self-contained, naturally rhythmic, evocative, original, and meaningful without surrounding context."
                        "Favor poetic, playful, surprising, or subtly mysterious language."
                        "Avoid generic clichés, awkward fragments, forced imagery, and associations with laziness or distraction."
                        "Don't penalize unusual grammar or incomplete sentences if they work artistically."
                        "A secondary objective is to have the phrase roughly evoke: Monday afternoon in October (autumn), sunny, mild."
)


def settings():
    values = {}
    env_path = Path(__file__).resolve().parent.parent / ".env"
    if env_path.exists():
        for raw_line in env_path.read_text().splitlines():
            line = raw_line.strip()
            if line.startswith("export "):
                line = line[7:].strip()
            name, separator, value = line.partition("=")
            if not separator:
                continue
            value = value.strip()
            if value[:1] in {"'", '"'}:
                value = value[1:].split(value[0], 1)[0]
            else:
                value = value.split("#", 1)[0].strip()
            values[name.strip()] = value
    return {
        "api_key": os.environ.get("OPENROUTER_API_KEY") or values.get("OPENROUTER_API_KEY", ""),
        "model": os.environ.get("JEV_MODEL") or values.get("JEV_MODEL", MODEL),
    }


def selected_samples():
    source = Path(__file__).with_name("03_random_sample.py")
    module = runpy.run_path(str(source), run_name="random_sample_module")
    return module["select_samples"]()[:255]


def main():
    config = settings()
    if not config["api_key"]:
        raise SystemExit("Set OPENROUTER_API_KEY in the environment or .env first.")

    samples = selected_samples()
    if not samples:
        raise SystemExit("No eligible rock or pop samples were selected.")

    criteria = {
        f"lyric_{index:02d}": sample["lyric"]
        for index, sample in enumerate(samples, start=1)
    }
    payload = {
        "model": config["model"],
        "state": STATE,
        "questions": {
            "best_lyric": {
                "type": "choice",
                "instructions": QUESTION_INSTRUCTIONS,
                "criteria": criteria,
            }
        },
    }
    request = Request(
        ENDPOINT,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {config['api_key']}",
            "Content-Type": "application/json",
            "X-Title": "Amber sample choice",
        },
        method="POST",
    )
    with urlopen(request, timeout=20) as response:
        result = json.loads(response.read())
    print(criteria)
    print(json.dumps(result, indent=2, ensure_ascii=False))
    probs = result["answers"]["best_lyric"]["probabilities"]
    outputs = {x: (probs[x], criteria[x]) for x in criteria}
    outputs = dict(sorted(outputs.items(), key=lambda item: item[1][0]))
    print(outputs)

if __name__ == "__main__":
    main()
