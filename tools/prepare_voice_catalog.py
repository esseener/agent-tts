"""Create a small, attributed catalog; names are UI aliases, not identities."""
from pathlib import Path
import hashlib
import json
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
SOURCE = "https://huggingface.co/datasets/AISHELL/AISHELL-3/resolve/466cda9f8c6677e6c25a80eb9bf5e989ea192d37/spk-info.txt"


def prepare():
    metadata = ROOT / ".build/downloads/aishell3-spk-info.txt"
    if not metadata.is_file():
        metadata.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(SOURCE, timeout=30) as response:
            metadata.write_bytes(response.read())
    people = {}
    for line in metadata.read_text(encoding="utf-8").splitlines():
        if line and not line.startswith("#"):
            speaker, age, gender, accent = line.split()
            people[speaker] = (age, gender, accent)
    female_first = list("知听若沐星清语晚初云舒安念映予")
    female_last = list("夏雨晴月禾溪音雪岚瑶宁秋澜晓")
    male_first = list("景云远清墨星泽怀承知")
    male_last = list("川舟辰言山林安明风然")
    counts = {"female": 0, "male": 0}
    voices = []
    for number, speaker in enumerate((ROOT / "resources/local-tts/speakers.txt").read_text().splitlines()):
        age, gender, accent = people[speaker]
        count = counts[gender]
        first, last = (female_first, female_last) if gender == "female" else (male_first, male_last)
        alias = first[count // len(last)] + last[count % len(last)]
        counts[gender] += 1
        voices.append({"id": number, "speaker": speaker, "name": alias, "gender": gender,
                       "age_group": age, "accent": accent,
                       "featured": number in [10, 33, 99, 21, 66, 41, 45, 103]})
    assert len(voices) == 174 and len({v["name"] for v in voices}) == 174
    catalog = {"source": SOURCE, "metadata_sha256": hashlib.sha256(metadata.read_bytes()).hexdigest(),
               "name_note": "中文名为 Agent Voice 界面昵称，不是真实姓名；性别和编号来自公开资料。",
               "voices": voices}
    (ROOT / "assets/voice-catalog.json").write_text(json.dumps(catalog, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Prepared {len(voices)} named speakers: {counts}")


if __name__ == "__main__":
    prepare()
