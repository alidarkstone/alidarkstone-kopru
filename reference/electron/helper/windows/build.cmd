@echo off
rem Kopru giris yardimcisini Windows'un kendi .NET Framework C# derleyicisiyle derler.
rem Cikti: helper\windows\bin\KopruInput.exe
setlocal
set CSC=%WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe
if not exist "%CSC%" set CSC=%WINDIR%\Microsoft.NET\Framework\v4.0.30319\csc.exe
if not exist "%CSC%" (
  echo .NET Framework 4 derleyicisi bulunamadi: %CSC%
  exit /b 1
)
if not exist "%~dp0bin" mkdir "%~dp0bin"
"%CSC%" /nologo /optimize+ /platform:anycpu /target:winexe /out:"%~dp0bin\KopruInput.exe" /r:System.Windows.Forms.dll /r:System.Drawing.dll /r:System.Web.Extensions.dll "%~dp0KopruInput.cs" || exit /b 1
echo Derlendi: %~dp0bin\KopruInput.exe
