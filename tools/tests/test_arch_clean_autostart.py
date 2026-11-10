"""Проверка guest-команды без доступа к systemd хоста."""

import ast
import subprocess
import unittest
from pathlib import Path


class NoAutostartTest(unittest.TestCase):
    def test_only_inactive_exit_three_is_accepted(self) -> None:
        source = Path(__file__).parents[1] / "arch_clean_vm_build.py"
        commands = [
            node.value.split("pacman -Qkk yonik; ", 1)[1]
            for node in ast.walk(ast.parse(source.read_text()))
            if isinstance(node, ast.Constant)
            and isinstance(node.value, str)
            and "pacman -Qkk yonik; " in node.value
            and "systemctl is-active" in node.value
        ]
        self.assertEqual(len(commands), 1)
        for state, status in [
            ("active", 0),
            ("inactive", 3),
            ("failed", 3),
            ("unknown", 4),
            ("error", 1),
        ]:
            with self.subTest(state=state, status=status):
                # Функция заменяет systemctl целиком, её аргументы передаёт wrapper ниже.
                script = (
                    f'systemctl() {{ printf "%s\\n" "{state}"; return {status}; }}; set -e; '
                    + commands[0]
                )
                result = subprocess.run(
                    ["/bin/bash", "-c", script],
                    capture_output=True,
                    timeout=2,
                    check=False,
                )
                self.assertEqual(result.returncode == 0, state == "inactive")


if __name__ == "__main__":
    unittest.main()
