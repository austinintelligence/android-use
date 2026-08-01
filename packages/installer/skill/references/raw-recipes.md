# Raw backend recipes

Use structured commands first. Raw commands are for an explicitly authorized gap:

```text
au adb -- shell getprop ro.serialno
au adb -g -- devices -l
au sh -- cmd package list packages
```

Arguments after `--` retain exact process boundaries. Raw shell is broadly unrestricted; it has no denylist and is never described as safe. Confirm the target/effect before destructive or privacy-sensitive raw commands and redirect potentially large output to a file.
