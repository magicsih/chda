#!/usr/bin/env python3
"""ABI and dependency-boundary regressions for Linux packages."""
import importlib.util
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("linux_package", pathlib.Path(__file__).with_name("package-linux.py"))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackageTests(unittest.TestCase):
    def test_host_graphics_and_c_runtime_stay_outside_the_payload(self):
        for name in ["libc.so.6", "libm.so.6", "ld-linux-x86-64.so.2", "libvulkan.so.1", "libGL.so.1", "libstdc++.so.6", "libgcc_s.so.1", "libwayland-client.so.0", "libxcb.so.1"]:
            self.assertTrue(package.host_library(name), name)
        for name in ["libxkbcommon.so.0", "libfontconfig.so.1"]:
            self.assertFalse(package.host_library(name), name)

    def test_abi_audit_uses_imports_and_compares_versions_numerically(self):
        symbols = """  1: 0000000000000000 0 FUNC GLOBAL DEFAULT UND memcpy@GLIBC_2.14 (2)
  2: 0000000000000000 0 FUNC GLOBAL DEFAULT UND pthread_create@GLIBC_2.31 (3)
  3: 0000000000000000 0 FUNC GLOBAL DEFAULT UND thing@GLIBCXX_3.4.28 (4)
  4: 0000000000000010 0 FUNC GLOBAL DEFAULT 12 exported@GLIBC_2.99
"""
        self.assertEqual(package.audit_symbols(symbols), {"GLIBC": "2.31", "GLIBCXX": "3.4.28"})
        for version in ["GLIBC_2.32", "GLIBC_2.100", "GLIBCXX_3.4.29", "GLIBC_PRIVATE"]:
            with self.assertRaises(ValueError):
                package.audit_symbols(f"1: 0 0 FUNC GLOBAL DEFAULT UND symbol@{version}")

    def test_missing_or_unexpected_dependencies_fail_staging(self):
        output = "libfoo.so.1 => /usr/lib/libfoo.so.1 (0x123)\nlibc.so.6 => /lib/libc.so.6 (0x456)\n/lib64/ld-linux-x86-64.so.2 (0x789)\nlinux-vdso.so.1 (0x000)\n"
        self.assertEqual(package.dependency_paths(output), {"libfoo.so.1": pathlib.Path("/usr/lib/libfoo.so.1")})
        for line in ["libfoo.so.1 => not found", "libfoo.so.1 => relative/libfoo.so.1 (0x123)"]:
            with self.assertRaises(ValueError):
                package.dependency_paths(line)


if __name__ == "__main__":
    unittest.main()
