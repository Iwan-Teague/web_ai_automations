# Output Layout

Runtime files are grouped by run.

```text
outputs/
  runs/
    run_YYYYMMDD_HHMMSS/
      summary.md
      transcript.md
      state_log.jsonl
      screenshots/
        debug_*.png
        template_miss_*.png
        model_picker_*.png
      results/
        task_1/
          iter_1_raw.md
          iter_1_artifacts/
            design_preview.png
            generated-project.zip
          iter_1_debate/
            round_1_bull.md
            round_1_bear.md
            round_1_judge.md
```

Legacy flat output from before the run-folder layout is archived under:

```text
outputs/legacy_flat_20260504/
```

Keep structural docs in `docs/`. Keep templates under `assets/<os>/<site>/templates/`. Do not put runtime screenshots, zips, transcripts, or copied AI responses in the repo root.

Matrix comparison assets are grouped by OS and site:

```text
assets/
  macos/
    arena.ai/
      templates/
        send_button_dark_100.png
        send_button_dark_100.rgbm
        copy_button_dark_100.png
        copy_button_dark_100.rgbm
      calibration/
        model_dropdown_<model>_selected.png
```

Future sites should follow the same shape, for example `assets/macos/example.com/templates/`.

PNG files are the inspectable source of truth. `.rgbm` files are decoded RGB matrices used as a faster cache. If a `.rgbm` is missing or older than the PNG, the loader rebuilds it from the PNG. To inspect a matrix cache:

```text
cargo run --bin matrix_to_png -- assets/macos/arena.ai/templates/copy_button_dark_100.rgbm /tmp/copy_button.png
```
