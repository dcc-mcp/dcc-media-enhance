"""Small in-memory PE fixtures; no Windows toolchain or binary download required."""
import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location('pe_checks', Path(__file__).resolve().parents[1] / 'scripts/ci/pe_checks.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
check = module.checked_windows_imports


def fixture(name='KERNEL32.dll', delay=False):
    data = bytearray(1536)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 0x3c, 0x80)
    data[0x80:0x84] = b'PE\0\0'
    struct.pack_into('<HH', data, 0x84, 0x8664, 1)
    struct.pack_into('<H', data, 0x94, 240)
    optional = 0x98
    struct.pack_into('<H', data, optional, 0x20b)
    struct.pack_into('<I', data, optional + 108, 16)
    struct.pack_into('<II', data, optional + 120, 0x1000, 40)
    struct.pack_into('<IIII', data, optional + 240 + 8, 1024, 0x1000, 1024, 512)
    struct.pack_into('<IIIII', data, 512, 0, 0, 0, 0x1080, 0)
    value = name.encode('ascii') + b'\0'
    data[640:640 + len(value)] = value
    if delay:
        struct.pack_into('<II', data, optional + 112 + 13 * 8, 0x1100, 64)
        struct.pack_into('<IIIIIIII', data, 768, 1, 0x1180, 0, 0, 0, 0, 0, 0)
        data[896:912] = b'VCRUNTIME140.dll\0'
    return data


class PeDependencyTests(unittest.TestCase):
    def test_normal_system_import(self):
        self.assertEqual(check(fixture()), ['KERNEL32.dll'])

    def test_runtime_families_rejected(self):
        for name in ['ucrtbase.dll', 'ucrtbased.dll', 'VCRUNTIME140.dll',
                     'MSVCP140.dll', 'msvcr120.dll', 'api-ms-win-crt-runtime-l1-1-0.dll']:
            with self.subTest(name=name), self.assertRaises(ValueError):
                check(fixture(name))

    def test_delay_imports_rejected(self):
        with self.assertRaisesRegex(ValueError, 'Delay-loaded'):
            check(fixture(delay=True))

    def test_malformed_metadata_rejected(self):
        mutations = [
            (0x80, '<I', 0),  # Signature
            (0x98 + 108, '<I', 2),  # Delay directory cannot be inspected
            (0x98 + 108, '<I', 100),  # Directory table outside optional header
            (0x98 + 124, '<I', 20),  # Missing zero descriptor
            (512 + 12, '<I', 0x1400),  # DLL name outside raw data
            (0x98 + 240 + 20, '<I', 2000),  # Raw section outside file
        ]
        for offset, fmt, value in mutations:
            data = fixture(); struct.pack_into(fmt, data, offset, value)
            with self.subTest(offset=offset, value=value), self.assertRaises(ValueError):
                check(data)

    def test_truncated_inputs_rejected(self):
        for size in [0, 2, 64, 200, 1000]:
            with self.subTest(size=size), self.assertRaises(ValueError):
                check(fixture()[:size])


if __name__ == '__main__':
    unittest.main()
