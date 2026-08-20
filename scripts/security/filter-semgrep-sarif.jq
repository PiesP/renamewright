.runs[].results |= map(
  select(any(.suppressions[]?; .kind == "inSource") | not)
)
