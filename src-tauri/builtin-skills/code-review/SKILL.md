---
name: code-review
description: "Use when reviewing code for bugs, security issues, performance problems, or adherence to best practices. Provides structured code review with severity levels and actionable feedback."
---

# Code Review

Provide thorough, structured code reviews with actionable feedback.

## Process

1. Read the diff or code changes
2. Analyze for: correctness, security, performance, maintainability, style
3. Report findings by severity:
   - **Critical**: Bugs, security holes, data loss risk
   - **Warning**: Performance issues, poor error handling, race conditions
   - **Info**: Style inconsistencies, naming suggestions, documentation gaps

## Guidelines

- Never rewrite code unless asked — point out issues and suggest fixes
- Focus on the diff, not the whole file
- If you see a pattern of issues, mention it once with a general recommendation
- Be specific: cite exact line numbers and code snippets
- Suggest concrete fixes, not vague advice