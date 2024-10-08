using System.Runtime.InteropServices;
using StarBreaker.Common;

namespace StarBreaker.Forge;

[StructLayout(LayoutKind.Sequential, Pack = 1)]
public readonly record struct DataForgeReference
{
    public readonly uint Item1;
    public readonly CigGuid Value;
}