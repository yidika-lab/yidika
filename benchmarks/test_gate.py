import unittest

import gate


class OverLimit(unittest.TestCase):
    def test_slower_than_the_limit_fails(self):
        self.assertTrue(gate.over_limit(1.2, 1.0, 1.15))

    def test_within_the_limit_passes(self):
        self.assertFalse(gate.over_limit(1.1, 1.0, 1.15))

    def test_runs_under_the_noise_floor_are_ignored(self):
        self.assertFalse(gate.over_limit(0.002, 0.001, 1.15, floor=0.02))


class Check(unittest.TestCase):
    def test_a_different_output_fails_even_when_faster(self):
        failures = gate.check("x", (0.1, 100, {b"1\n"}), (0.2, 100, {b"2\n"}))
        self.assertEqual(failures, ["x: output differs from C"])

    def test_more_memory_than_allowed_fails(self):
        failures = gate.check("x", (0.1, 300, {b"1\n"}), (0.1, 100, {b"1\n"}))
        self.assertEqual(len(failures), 1)
        self.assertIn("KB", failures[0])

    def test_an_output_that_changes_between_runs_fails(self):
        failures = gate.check("x", (0.1, 100, {b"1\n", b"2\n"}), (0.1, 100, {b"1\n"}))
        self.assertEqual(failures, ["x: output differs from C"])


class Render(unittest.TestCase):
    def test_the_report_shows_both_binary_sizes(self):
        report = gate.render([("x", (0.1, 100, {b"1\n"}), (0.1, 100, {b"1\n"}), 9216, 16384)], [], [])
        self.assertIn("| x | 0.100 s | 0.100 s | 1.00 | 100 KB | 100 KB | 9 KB | 16 KB |", report)

    def test_the_report_lists_the_linked_libraries(self):
        report = gate.render([], [], ["libc.so.6"])
        self.assertIn("Libraries linked by a Yidika program: libc.so.6", report)


if __name__ == "__main__":
    unittest.main()
