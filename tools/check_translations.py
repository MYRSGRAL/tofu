#!/usr/bin/env python3
"""Строгая проверка встроенных пар RU/EN в текущем QML GUI без Qt."""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = Path("crates/yonik-desktop/qml/Main.qml")
TOKEN = re.compile(
    r'\s+|//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|'
    r"'(?:\\.|[^'\\])*'|[A-Za-z_$][\w$]*|\d+|[^\s]"
)
PLACEHOLDER = re.compile(r"%L?\d+|%n|\$\{[^{}]+\}|(?<!\{)\{[^{}]*\}(?!\})")


class CatalogError(ValueError):
    """Каталог не соответствует поддерживаемому строгому формату."""


def tokens(source: str) -> list[str]:
    """Убирает комментарии, сохраняя строки как неделимые токены."""
    return [
        match.group()
        for match in TOKEN.finditer(source)
        if not match.group().isspace() and not match.group().startswith(("//", "/*"))
    ]


def literal(token: str) -> str:
    """Декодирует используемый каталогом JSON-совместимый строковый литерал."""
    try:
        value = json.loads(token)
    except (ValueError, TypeError) as error:
        raise CatalogError(f"Ожидался строковый литерал: {token}") from error
    if not isinstance(value, str):
        raise CatalogError(f"Ожидалась строка: {token}")
    return value


def validate_pair(key: str, ru: str, en: str) -> None:
    """Проверяет наличие обоих языков и мультимножество placeholders."""
    if not ru.strip() or not en.strip():
        raise CatalogError(f"{key}: отсутствующий или пустой перевод RU/EN")
    if Counter(PLACEHOLDER.findall(ru)) != Counter(PLACEHOLDER.findall(en)):
        raise CatalogError(f"{key}: несовпадение placeholders RU/EN")


def check_source(source: str) -> tuple[int, int]:
    """Разбирает tr-вызовы и карты объявлений; неизвестные формы отклоняет."""
    ts = tokens(source)
    inline = 0
    keys = 0
    maps = 0
    index = 0
    while index < len(ts):
        declaration = ts[index] in {"const", "let", "var"}
        initializer = index + 3
        if declaration and ts[index + 2 : index + 3] == ["="]:
            while ts[initializer : initializer + 1] == ["("]:
                initializer += 1
            if initializer > index + 3 and ts[initializer : initializer + 1] == ["{"]:
                raise CatalogError("Карта в скобках: используйте прямой объект")
        if declaration and ts[index + 2 : index + 4] == ["=", "{"]:
            name = ts[index + 1]
            maps += 1
            index += 4
            seen: set[str] = set()
            while index < len(ts) and ts[index] != "}":
                if ts[index].startswith("'"):
                    raise CatalogError(f"{name}: ключ в неподдерживаемых кавычках")
                key = literal(ts[index]) if ts[index].startswith('"') else ts[index]
                if key in seen:
                    raise CatalogError(f"{name}: повторный ключ {key}")
                seen.add(key)
                if ts[index + 1 : index + 3] != [":", "["]:
                    raise CatalogError(f"{name}.{key}: ожидалась пара [RU, EN]")
                if ts[index + 4 : index + 5] != [","] or ts[index + 6 : index + 7] != [
                    "]"
                ]:
                    raise CatalogError(
                        f"{name}.{key}: отсутствует язык или лишний элемент"
                    )
                validate_pair(
                    f"{name}.{key}", literal(ts[index + 3]), literal(ts[index + 5])
                )
                keys += 1
                index += 7
                if ts[index : index + 1] == [","]:
                    index += 1
                elif ts[index : index + 1] != ["}"]:
                    raise CatalogError(f"{name}: ожидалась запятая")
            if not seen:
                raise CatalogError(f"{name}: пустая карта переводов")
            if index >= len(ts):
                raise CatalogError(f"{name}: незакрытая карта")
        elif ts[index : index + 2] == ["tr", "("]:
            end = index + 2
            while end < len(ts) and ts[end] != ")":
                end += 1
            args = ts[index + 2 : end]
            if index > 0 and ts[index - 1] == "function":
                if args != ["ru", ",", "en"]:
                    raise CatalogError("Изменён контракт функции tr")
            elif len(args) == 3 and args[1] == ",":
                validate_pair(f"tr #{inline + 1}", literal(args[0]), literal(args[2]))
                inline += 1
            elif args not in [
                [name, "[", "0", "]", ",", name, "[", "1", "]"]
                for name in ("pair", "reason", "field")
            ]:
                raise CatalogError(f"Неподдерживаемый динамический tr: {args}")
            index = end
        index += 1
    if not inline or not maps or not keys:
        raise CatalogError("Пустой каталог или изменён механизм переводов")
    return inline, keys


def main() -> int:
    """Возвращает ненулевой код при любой ошибке чтения или проверки."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    try:
        inline, keys = check_source((args.root / CATALOG).read_text(encoding="utf-8"))
    except (OSError, ValueError, IndexError) as error:
        parser.exit(1, f"Ошибка переводов: {error}\n")
    print(f"RU/EN: проверено {inline} inline-пар и {keys} ключей карт")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
