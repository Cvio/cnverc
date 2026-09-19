The translation model goes here: exactly one .gguf file, and it must be a
Qwen3 model. The one cnverc is set up for is

    qwen3-1.7b-q4_k_m.gguf

See ../../README.md, step 6, for where to download it.

cnverc uses whichever .gguf it finds in this folder. If there are two, it
stops and asks you to remove one rather than guessing.
