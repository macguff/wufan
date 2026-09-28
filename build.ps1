param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $XtaskArgs
)

cargo run --locked -p xtask -- @XtaskArgs
exit $LASTEXITCODE

