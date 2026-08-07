@echo off
echo Starting download via gh-proxy.com...
"C:\Program Files\Git\mingw64\bin\curl.exe" -L -C - --connect-timeout 30 --max-time 1800 --retry 3 --retry-delay 5 -o "C:\Users\鬼斩\mingw-w64.zip" "https://gh-proxy.com/https://github.com/brechtsanders/winlibs_mingw/releases/download/16.1.0posix-14.0.0-ucrt-r4/winlibs-x86_64-posix-seh-gcc-16.1.0-mingw-w64ucrt-14.0.0-r4.zip"
echo Download exit code: %ERRORLEVEL%
echo DONE > "C:\Users\鬼斩\download_complete.txt"
