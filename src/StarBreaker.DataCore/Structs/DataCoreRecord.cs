using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using StarBreaker.Common;

namespace StarBreaker.DataCore;

/// <summary>V6 record layout (32 bytes). Used only for deserialization of older DCB files.</summary>
[StructLayout(LayoutKind.Sequential, Pack = 1)]
internal readonly record struct DataCoreRecordV6
{
    public readonly DataCoreStringId2 NameOffset;
    public readonly DataCoreStringId FileNameOffset;
    public readonly int StructIndex;
    public readonly CigGuid Id;
    public readonly ushort InstanceIndex;
    public readonly ushort StructSize;
}

/// <summary>Record layout (36 bytes, v8+). V6 records are converted with TagOffset.Id = -1.</summary>
[StructLayout(LayoutKind.Sequential, Pack = 1)]
public readonly record struct DataCoreRecord
{
    public readonly DataCoreStringId2 NameOffset;
    public readonly DataCoreStringId FileNameOffset;
    public readonly DataCoreStringId2 TagOffset;
    public readonly int StructIndex;
    public readonly CigGuid Id;
    public readonly ushort InstanceIndex;
    public readonly ushort StructSize;

    internal static DataCoreRecord FromV6(in DataCoreRecordV6 r)
    {
        Unsafe.SkipInit(out DataCoreRecord result);
        ref var dst = ref Unsafe.AsRef(in result);
        Unsafe.As<DataCoreRecord, DataCoreStringId2>(ref dst) = r.NameOffset;
        Unsafe.Add(ref Unsafe.As<DataCoreRecord, int>(ref dst), 1) = r.FileNameOffset.Id;
        Unsafe.Add(ref Unsafe.As<DataCoreRecord, int>(ref dst), 2) = -1; // TagOffset = -1
        Unsafe.Add(ref Unsafe.As<DataCoreRecord, int>(ref dst), 3) = r.StructIndex;
        // Copy guid + instance_index + struct_size (20 bytes from offset 16)
        Unsafe.CopyBlock(
            ref Unsafe.As<CigGuid, byte>(ref Unsafe.AsRef(in result.Id)),
            ref Unsafe.As<CigGuid, byte>(ref Unsafe.AsRef(in r.Id)),
            20);
        return result;
    }

    public string GetName(DataCoreDatabase db) => db.GetString2(NameOffset);
    public string GetFileName(DataCoreDatabase db) => db.GetString(FileNameOffset);
    public string? GetTag(DataCoreDatabase db) => TagOffset.Id != -1 ? db.GetString2(TagOffset) : null;

#if DEBUG
    public string Name => DebugGlobal.Database.GetString2(NameOffset);
    public string FileName => DebugGlobal.Database.GetString(FileNameOffset);
    public DataCoreStructDefinition Struct => DebugGlobal.Database.StructDefinitions[StructIndex];
#endif
}