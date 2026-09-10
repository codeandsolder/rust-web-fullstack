from pathlib import Path

path = Path('.github/workflows/maintenance-queue.yml')
text = path.read_text()
for token in (
    '`cargo upgrade`',
    '`cargo update`',
    '`Cargo.toml`',
    '`Cargo.lock`',
    '`SKILL.md`',
):
    text = text.replace(token, token.replace('`', r'\`'))
path.write_text(text)
