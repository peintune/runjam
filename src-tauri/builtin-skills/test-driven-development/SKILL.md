---
name: test-driven-development
description: "Use when implementing any feature or bugfix, before writing implementation code. Enforces red-green-refactor cycle with strict rules."
---

# Test-Driven Development (TDD)

## Rule

Write the test first. Watch it fail. Write minimal code to pass.

**Core principle:** If you didn't watch the test fail, you don't know if it tests the right thing.

## When to Use

Always for:
- New features
- Bug fixes
- Refactoring with behavior changes

Skip only for:
- Config file changes
- Documentation updates
- Non-functional changes (formatting, comments)

## Process

1. **Red**: Write a failing test
2. **Green**: Write minimal code to pass
3. **Refactor**: Clean up while keeping tests green

## Anti-Patterns

- Writing implementation before the test
- Skipping the "watch it fail" step
- Writing tests that are too coupled to implementation
- Testing framework behavior instead of your code