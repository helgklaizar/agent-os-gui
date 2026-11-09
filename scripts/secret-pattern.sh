# Single source of the secret-detection regex (sourced by the pre-commit hook and scripts/check.sh).
# shellcheck disable=SC2034
SECRET_PATTERN='(sk-[A-Za-z0-9_-]{20,}|AKIA[0-9A-Z]{16}|BEGIN (RSA|OPENSSH|EC|PGP) PRIVATE KEY|xox[baprs]-[A-Za-z0-9-]{10,}|ghp_[A-Za-z0-9]{30,}|glpat-[A-Za-z0-9_-]{20,})'
