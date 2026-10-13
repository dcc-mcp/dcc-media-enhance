"""Bounded PE32+ dependency gate for the native Windows distribution.

Delay loading is not used by this project, so reject it rather than claiming a
complete dependency inventory for a PE format feature we do not parse.
"""
import struct


def checked_windows_imports(data):
    def require(condition, message):
        if not condition:
            raise ValueError(message)

    def unpack(fmt, offset):
        size = struct.calcsize(fmt)
        require(0 <= offset <= len(data) - size, 'Truncated PE field')
        return struct.unpack_from(fmt, data, offset)

    require(data[:2] == b'MZ', 'Windows artifact is not PE')
    pe, = unpack('<I', 0x3c)
    require(pe >= 0x40 and data[pe:pe + 4] == b'PE\0\0', 'Invalid PE signature')
    machine, count = unpack('<HH', pe + 4)
    size, = unpack('<H', pe + 20)
    optional = pe + 24
    require(machine == 0x8664 and unpack('<H', optional)[0] == 0x20b,
            'Expected x86_64 PE32+')
    require(size >= 112 + 14 * 8 and optional + size <= len(data),
            'Missing PE data directories')
    directories, = unpack('<I', optional + 108)
    require(14 <= directories <= (size - 112) // 8, 'Invalid PE directory count')
    delay = unpack('<II', optional + 112 + 13 * 8)
    require(delay == (0, 0), 'Delay-loaded dependencies are not supported')
    table = optional + size
    require(count > 0 and table + count * 40 <= len(data), 'Truncated PE section table')
    sections = [unpack('<IIII', table + i * 40 + 8) for i in range(count)]

    def mapped(rva, size=1):
        for virtual_size, virtual, raw_size, raw in sections:
            if virtual <= rva < virtual + max(virtual_size, raw_size):
                delta = rva - virtual
                require(size <= raw_size - delta and raw + raw_size <= len(data),
                        'PE RVA outside section file bytes')
                return raw + delta, raw + raw_size
        raise ValueError('PE RVA outside sections')

    rva, directory_size = unpack('<II', optional + 112 + 8)
    require(rva != 0 and directory_size >= 20, 'Missing PE import directory')
    pos, _ = mapped(rva, directory_size)
    end = pos + directory_size
    imports = []
    while pos + 20 <= end:
        descriptor = unpack('<IIIII', pos)
        if not any(descriptor):
            require(bool(imports), 'Empty PE import inventory')
            forbidden = ('vcruntime', 'msvcp', 'msvcr', 'api-ms-win-crt-',
                         'ucrtbase', 'ucrtbased')
            require(not any(name.lower().startswith(forbidden) for name in imports),
                    f'External C/C++ runtime dependency: {imports}')
            return imports
        name_pos, name_end = mapped(descriptor[3])
        zero = data.find(b'\0', name_pos, name_end)
        require(zero > name_pos, 'Unterminated or empty PE import name')
        name = data[name_pos:zero].decode('ascii')
        require('/' not in name and '\\' not in name and ':' not in name,
                'Unexpected PE import path')
        imports.append(name)
        pos += 20
    raise ValueError('Unterminated PE import directory')
