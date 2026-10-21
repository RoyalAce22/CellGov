"""Write the spu_reference_float capture into the reference fixtures.

For each vector in vectors.tsv, adds a vector named "<name>-console" to
its unit's fixture, with hardware_capture provenance naming this
capture. A console vector keeps the documented vector's words, start
state and step limit, and fills each component the documented vector
leaves undefined from the capture:

  regs_hex            RT as the console left it
  fpscr               the FPSCR the console read after the words
  end, pc, stop and   the console ran past the words: the end is the step
  interrupts_enabled  limit, the PC follows the last word, no stop, and
                      interrupts stay disabled

A vector with no documented counterpart (an EXTRA of gen_vectors.py)
takes every other component from its unit's first vector. The
documented vector's undefined reasons point at the console vector.

Run from this directory after a capture: python3 apply_capture.py
Running it again replaces the console vectors it wrote before.
"""

import hashlib
import json
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE / '../../../crates/cellgov_fuzz/tests/fixtures/spu_reference'
CAPTURE = HERE / 'ps3/cech20-cex-493'
SUFFIX = '-console'
OLD_REASON_TAIL = 'left to hardware capture, issue #1411'
CHANNELS_REASON = 'the capture records no channel state, which the runtime refreshes at each step'


def frame_words():
    data = (CAPTURE / 'cgov_frame.bin').read_bytes()
    at = data.find(b'CGOV')
    n = int.from_bytes(data[at + 4:at + 8], 'big')
    return data[at + 8:at + 8 + n]


def provenance():
    digest = hashlib.sha256((CAPTURE / 'cgov_frame.bin').read_bytes()).hexdigest()
    return {
        'kind': 'hardware_capture',
        'capture_id': 'spu_reference_float/cech20-cex-493',
        'device': 'CECH-2001A',
        'environment': 'firmware 4.93, EvilNAT 4.93 PEX, SPU thread group',
        'source_sha256': digest,
    }


def undefined(component):
    return isinstance(component, dict) and component.get('status') == 'undefined'


def main():
    body = frame_words()
    rows = (HERE / 'vectors.tsv').read_bytes().decode().splitlines()[1:]
    prov = provenance()
    docs = {}
    for row in rows:
        index, unit, name, rt, words, regs, fpscr = row.split('\t')
        index, rt = int(index), int(rt)
        rt_hex = body[32 * index:32 * index + 16].hex()
        fpscr_hex = body[32 * index + 16:32 * index + 32].hex()
        if unit not in docs:
            doc = json.loads((FIXTURES / (unit + '.json')).read_bytes())
            doc['vectors'] = [v for v in doc['vectors'] if not v['name'].endswith(SUFFIX)]
            docs[unit] = doc
        doc = docs[unit]
        by_name = {v['name']: v for v in doc['vectors']}
        documented = by_name.get(name)
        if documented is not None:
            console = json.loads(json.dumps(documented))
            for component in documented['expected'].values():
                if undefined(component) and component['reason'].endswith(OLD_REASON_TAIL):
                    # Keep the rule, the clause before '; ', and replace the
                    # whole clause that pointed at a future capture.
                    rule = component['reason'].rsplit('; ', 1)[0]
                    component['reason'] = (rule + '; the %s%s vector holds the console\'s answer'
                                           % (name, SUFFIX))
        else:
            console = json.loads(json.dumps(doc['vectors'][0]))
            start = {}
            for pair in regs.split(','):
                r, value = pair.split('=')
                start[r] = value
            console['initial_state'] = {'regs_hex': start, 'local_store': [], 'pc': 0}
        console['name'] = name + SUFFIX
        console['provenance'] = prov
        expected = console['expected']
        if expected['channels'].get('status') == 'unsupported':
            expected['channels'] = {'status': 'unsupported', 'reason': CHANNELS_REASON}
        undef = {k for k, c in (documented or console)['expected'].items() if undefined(c)}
        if documented is None:
            undef = {'regs_hex', 'fpscr'}
        if 'regs_hex' in undef:
            expected['regs_hex'] = {'status': 'value', 'value': {str(rt): rt_hex}}
        if 'fpscr' in undef:
            expected['fpscr'] = {'status': 'value', 'value': fpscr_hex}
        if 'end' in undef:
            expected['end'] = {'status': 'value', 'value': {'state': 'step_limit'}}
        if 'pc' in undef:
            start_pc = console['initial_state'].get('pc', 0)
            expected['pc'] = {'status': 'value', 'value': start_pc + 4 * len(words.split(','))}
        if 'stop' in undef:
            expected['stop'] = {'status': 'value', 'value': None}
        if 'interrupts_enabled' in undef:
            expected['interrupts_enabled'] = {'status': 'value', 'value': False}
        left = sorted(k for k, c in expected.items() if undefined(c))
        assert not left, (unit, name, left)
        doc['vectors'].append(console)
    for unit, doc in docs.items():
        assert len(doc['vectors']) <= 64, unit
        (FIXTURES / (unit + '.json')).write_bytes((json.dumps(doc, indent=2) + '\n').encode())
    print('%d console vectors in %d fixtures' % (len(rows), len(docs)))


if __name__ == '__main__':
    main()
