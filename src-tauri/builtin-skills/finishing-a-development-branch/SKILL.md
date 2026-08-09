---
name: finishing-a-development-branch
description: "Use when development work is complete and you need to prepare for merge. Guides through cleanup, testing, documentation, and creating a well-structured pull request."
---

# Finishing a Development Branch

## Checklist

1. **Clean up**: Remove debug logs, commented-out code, TODO markers
2. **Test**: Run full test suite, verify edge cases
3. **Lint**: Run linter and formatter
4. **Document**: Update README, changelog, or inline docs if needed
5. **Review diff**: Self-review the full diff before pushing
6. **Commit**: Use descriptive commit messages (what + why, not how)
7. **Push**: Push branch and create PR with clear description

## PR Description Template

```
## What
Brief description of the change

## Why
Why this change is needed

## How
High-level approach (not implementation details)

## Testing
How to verify the change works
```