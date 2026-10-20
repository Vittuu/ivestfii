"""Exercise native storage with a synthetic fixture or an explicit local legacy JSON.

Only counts and integrity checks are printed; portfolio values stay in ignored
local files. Run after building ivestfii-desktop with custom-protocol.
"""
from decimal import Decimal
import argparse
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import sys
import uuid


root = Path(__file__).resolve().parent.parent
arguments = argparse.ArgumentParser(description=__doc__)
arguments.add_argument('--binary', type=Path, help='Executável desktop alternativo, como o exemplo ivestfii-preview.')
arguments.add_argument('--source', type=Path, help='JSON local a conferir; por padrão usa o exemplo fictício versionado.')
options = arguments.parse_args()
source = options.source.resolve() if options.source else root / 'examples' / 'portfolio.example.json'
binary = options.binary.resolve() if options.binary else root / 'target' / 'debug' / 'ivestfii-desktop.exe'
if not binary.is_file():
    raise SystemExit('Compile antes: cargo build -p ivestfii-desktop --features custom-protocol')
directory = root / '.local' / f'verification-{uuid.uuid4().hex}'
directory.mkdir(parents=True)
original = source.read_bytes()
fingerprint = sha256(original).hexdigest()


def run_native():
    result = subprocess.run(
        [str(binary), '--verify-storage', '--data-dir', str(directory), '--legacy-file', str(source)],
        capture_output=True, text=True, encoding='utf-8', timeout=30,
        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
    )
    if result.returncode:
        raise RuntimeError(f'Verificação nativa falhou: {result.stderr.strip()}')
    return json.loads(result.stdout)


def canonical(data):
    result = []
    numeric = ('cotas_added', 'price_per_cota', 'dividend_per_cota', 'dividend_total')
    for fund in data['fiis']:
        entries = []
        for entry in fund.get('entries', []):
            normalized = {'month': entry['month'], 'notes': entry.get('notes', '')}
            for key in numeric:
                value = entry.get(key)
                normalized[key] = None if value is None else Decimal(str(value))
            entries.append(normalized)
        result.append({
            'ticker': fund['ticker'].upper(), 'name': fund.get('name', ''),
            'sector': fund.get('sector', ''), 'entries': sorted(entries, key=lambda item: item['month']),
        })
    return sorted(result, key=lambda item: item['ticker'])


first = run_native()
second = run_native()
if first['migration']['alreadyImported'] or not second['migration']['alreadyImported']:
    raise AssertionError('A importação não foi idempotente.')
if first['fundCount'] != second['fundCount'] or first['recordCount'] != second['recordCount']:
    raise AssertionError('As contagens mudaram na segunda importação.')
export = json.loads((directory / 'verified-export.json').read_text(encoding='utf-8'), parse_float=Decimal)
if canonical(json.loads(original, parse_float=Decimal)) != canonical(export):
    raise AssertionError('A exportação alterou campos ou valores da carteira.')
if source.read_bytes() != original:
    raise AssertionError('O arquivo original foi alterado.')
backup = Path(first['migration']['backupPath'])
if sha256(backup.read_bytes()).hexdigest() != fingerprint:
    raise AssertionError('O backup não corresponde ao original.')
report = {
    'status': 'ok', 'fundCount': first['fundCount'], 'recordCount': first['recordCount'],
    'originalUnchanged': True, 'backupExact': True, 'exportEquivalent': True,
    'idempotent': True, 'sha256': fingerprint, 'dataDirectory': str(directory),
    'sourcePath': str(source),
}
(root / '.local' / 'last-verification.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps({key: value for key, value in report.items() if key not in ('dataDirectory', 'sourcePath')}))
