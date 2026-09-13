#!/bin/sh
E=$(printf '\033')
p() { printf '%s' "$1"; }
p "$E[?3h$E[2J$E[H$E[?25l"
p "$E#6$E[1m 132 COLUMN MODE$E[m"
p "$E[3;1H----+----1----+----2----+----3----+----4----+----5----+----6----+----7----+----8----+----9----+---10----+---11----+---12----+---13--"
i=5
for line in \
"Directory DKA0:[SYS0.SYSCOMMON.SYSEXE]" "" \
"AUTHORIZE.EXE;1          248/252        4-OCT-1990 11:02:17.44  [SYSTEM]    (RWED,RWED,RE,RE)" \
"BACKUP.EXE;1             888/888        4-OCT-1990 11:02:21.16  [SYSTEM]    (RWED,RWED,RE,RE)" \
"DCL.EXE;1                612/612        4-OCT-1990 11:02:30.08  [SYSTEM]    (RWED,RWED,RE,RE)" \
"EDT.EXE;1                280/280        4-OCT-1990 11:02:34.61  [SYSTEM]    (RWED,RWED,RE,RE)" \
"LOGINOUT.EXE;1           176/176        4-OCT-1990 11:02:41.93  [SYSTEM]    (RWED,RWED,RE,RE)" \
"MAIL.EXE;1               540/540        4-OCT-1990 11:02:45.20  [SYSTEM]    (RWED,RWED,RE,RE)" \
"MONITOR.EXE;1            364/364        4-OCT-1990 11:02:49.77  [SYSTEM]    (RWED,RWED,RE,RE)" \
"TPU.EXE;1                408/408        4-OCT-1990 11:02:53.02  [SYSTEM]    (RWED,RWED,RE,RE)" \
"" "Total of 8 files, 3516/3528 blocks."; do
  p "$E[$i;1H$line"; i=$((i+1)); done
p "$E[20;1H$E#3  Heritage, in full.$E[21;1H$E#4  Heritage, in full."
sleep 30
