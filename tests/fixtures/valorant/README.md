# Synthetic VALORANT detector fixtures

These PNGs are original test graphics, not screenshots or extracted game assets.
They use a plain dark background, invented player names, simple score/killfeed
text and an explicit synthetic-footage label. They were rendered at 1280×720
with Open Sans; no font files are distributed here.

The automatic-highlight regression test embeds these images and creates a video
with FFmpeg: empty feed, three own kills, a score/round transition, empty feed,
then two more own kills. It also generates three tagged audio streams. The
fixture is intentionally deterministic and contains no real player data.

Passing these fixtures verifies integration, grouping and exported media. It is
not evidence for detection accuracy in real VALORANT footage.
