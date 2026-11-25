import csv, io, os, zipfile
from pathlib import Path

out = Path(__file__).resolve().parent

rows = [
    ["1001", "Coffee shop",  "01/02/25", "Debit",  "-$4.25",    "$1,245.75", "$4.25",    "A-1"],
    ["1001", "Payroll",      "01/03/25", "Credit", "$2,000.00", "$3,245.75", "-$2,000.00", "B-2"],
    ["1001", "Rent",         "01/04/25", "Debit",  "($1,200.00)", "$2,045.75", "$1,200.00", "C-3"],
]
header = ["Account", "Description", "Day", "Type", "Amount", "Balance", "Derived", "Note"]

# CSV
with open(out / 'transactions.csv', 'w', newline='', encoding='utf-8') as f:
    writer = csv.writer(f)
    writer.writerow(header)
    writer.writerows(rows)

# TSV
with open(out / 'transactions.tsv', 'w', newline='', encoding='utf-8') as f:
    writer = csv.writer(f, delimiter='\t')
    writer.writerow(header)
    writer.writerows(rows)

# XLSX
def col(i):
    name = ''
    i += 1
    while i:
        i, rem = divmod(i - 1, 26)
        name = chr(65 + rem) + name
    return name

def inline(ref, text):
    return f'<c r="{ref}" t="inlineStr"><is><t>{text}</t></is></c>'

def number(ref, value, style=None):
    s = f' s="{style}"' if style is not None else ''
    return f'<c r="{ref}"{s}><v>{value}</v></c>'

def formula(ref, expression, value, style=None):
    s = f' s="{style}"' if style is not None else ''
    return f'<c r="{ref}"{s}><f>{expression}</f><v>{value}</v></c>'

sheet_rows = ['<row r="1">' + ''.join(inline(f'{col(i)}1', h) for i, h in enumerate(header)) + '</row>']
serials = [45659, 45660, 45661]
for r, (account, desc, _day, typ, amount, balance, derived, note) in enumerate(rows, start=2):
    cells = [
        number(f'A{r}', account),
        inline(f'B{r}', desc),
        number(f'C{r}', serials[r - 2], 1),
        inline(f'D{r}', typ),
        number(f'E{r}', amount.replace('$', '').replace(',', '').replace('(', '-').replace(')', '')),
        number(f'F{r}', balance.replace('$', '').replace(',', '')),
        formula(f'G{r}', f'-E{r}', derived.replace('$', '').replace(',', '').replace('(', '-').replace(')', ''), 2),
        inline(f'H{r}', note),
    ]
    sheet_rows.append(f'<row r="{r}">' + ''.join(cells) + '</row>')

content_types = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>'''

rels = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>'''

workbook = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Transactions" sheetId="1" r:id="rId1"/></sheets>
</workbook>'''

workbook_rels = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>'''

styles = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<numFmts count="1"><numFmt numFmtId="164" formatCode="$#,##0.00;($#,##0.00)"/></numFmts>
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border/></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="3">
<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
<xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
</cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>'''

sheet = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>{''.join(sheet_rows)}</sheetData>
</worksheet>'''

with zipfile.ZipFile(out / 'transactions.xlsx', 'w', zipfile.ZIP_DEFLATED) as z:
    z.writestr('[Content_Types].xml', content_types)
    z.writestr('_rels/.rels', rels)
    z.writestr('xl/workbook.xml', workbook)
    z.writestr('xl/_rels/workbook.xml.rels', workbook_rels)
    z.writestr('xl/styles.xml', styles)
    z.writestr('xl/worksheets/sheet1.xml', sheet)

# ODS
def ods_text(text):
    return f'<table:table-cell office:value-type="string"><text:p>{text}</text:p></table:table-cell>'

def ods_float(value):
    return f'<table:table-cell office:value-type="float" office:value="{value}"/>'

def ods_date(value):
    return f'<table:table-cell office:value-type="date" office:date-value="{value}"/>'

def ods_currency(value):
    return f'<table:table-cell office:value-type="currency" office:currency="USD" office:value="{value}"/>'

def ods_formula(expression, value):
    return (f'<table:table-cell table:formula="of:={expression}" office:value-type="currency" '
            f'office:currency="USD" office:value="{value}"><text:p>{value}</text:p></table:table-cell>')

ods_rows = ['<table:table-row>' + ''.join(ods_text(h) for h in header) + '</table:table-row>']
dates = ['2025-01-02', '2025-01-03', '2025-01-04']
for r, (account, desc, _day, typ, amount, balance, derived, note) in enumerate(rows, start=2):
    amount_value = amount.replace('$', '').replace(',', '').replace('(', '-').replace(')', '')
    balance_value = balance.replace('$', '').replace(',', '')
    derived_value = derived.replace('$', '').replace(',', '').replace('(', '-').replace(')', '')
    ods_rows.append('<table:table-row>' + ''.join([
        ods_float(account),
        ods_text(desc),
        ods_date(dates[r - 2]),
        ods_text(typ),
        ods_currency(amount_value),
        ods_currency(balance_value),
        ods_formula(f'-[.E{r}]', derived_value),
        ods_text(note),
    ]) + '</table:table-row>')

content = f'''<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.3">
<office:body><office:spreadsheet>
<table:table table:name="Transactions">{''.join(ods_rows)}</table:table>
</office:spreadsheet></office:body>
</office:document-content>'''

manifest = '''<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>'''

with zipfile.ZipFile(out / 'transactions.ods', 'w') as z:
    z.writestr('mimetype', 'application/vnd.oasis.opendocument.spreadsheet', compress_type=zipfile.ZIP_STORED)
    z.writestr('META-INF/manifest.xml', manifest, compress_type=zipfile.ZIP_DEFLATED)
    z.writestr('content.xml', content, compress_type=zipfile.ZIP_DEFLATED)

print('wrote', sorted(p.name for p in out.iterdir()))

# SQLite and the SQL dump that builds it. Dates are ISO text with a DATE declared type, which is
# how SQLite stores them; the importer reads the declared type and parses the value.
import sqlite3

sqlite_rows = [
    (1001, "Coffee shop", "2025-01-02", "Debit", -4.25, 1245.75, 4.25, "A-1"),
    (1001, "Payroll", "2025-01-03", "Credit", 2000.0, 3245.75, -2000.0, "B-2"),
    (1001, "Rent", "2025-01-04", "Debit", -1200.0, 2045.75, 1200.0, "C-3"),
]
schema = ("CREATE TABLE transactions (Account INTEGER, Description TEXT, Day DATE, Type TEXT, "
          "Amount REAL, Balance REAL, Derived REAL, Note TEXT);")

db = out / 'transactions.sqlite'
if db.exists():
    db.unlink()
conn = sqlite3.connect(db)
conn.execute(schema)
conn.executemany('INSERT INTO transactions VALUES (?,?,?,?,?,?,?,?)', sqlite_rows)
conn.commit()
conn.close()

def sql_literal(value):
    if value is None:
        return 'NULL'
    if isinstance(value, str):
        return "'" + value.replace("'", "''") + "'"
    return str(value)

with open(out / 'transactions.sql', 'w', encoding='utf-8') as f:
    f.write('BEGIN TRANSACTION;\n')
    f.write(schema + '\n')
    for row in sqlite_rows:
        f.write('INSERT INTO transactions VALUES (' + ', '.join(sql_literal(v) for v in row) + ');\n')
    f.write('COMMIT;\n')

print('wrote', sorted(p.name for p in out.iterdir()))
