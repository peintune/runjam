@echo off
echo Extracting...
powershell -ExecutionPolicy Bypass -NoProfile -Command "Expand-Archive -Path 'C:\Users\鬼斩\mingw-w64.zip' -DestinationPath 'C:\Users\鬼斩\mingw-w64' -Force"
echo Extraction exit code: %ERRORLEVEL%
echo DONE > "C:\Users\鬼斩\extract_complete.txt"
