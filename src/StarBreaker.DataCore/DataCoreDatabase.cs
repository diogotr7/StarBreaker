using System.Collections.Frozen;
using System.Text;
using StarBreaker.Common;

namespace StarBreaker.DataCore;

public sealed class DataCoreDatabase
{
    private readonly int DataSectionOffset;
    private readonly byte[] DataSection;

    public readonly DataCoreStructDefinition[] StructDefinitions;
    public readonly DataCorePropertyDefinition[] PropertyDefinitions;
    public readonly DataCoreEnumDefinition[] EnumDefinitions;
    public readonly DataCoreDataMapping[] DataMappings;
    public readonly DataCoreRecord[] RecordDefinitions;

    public readonly sbyte[] Int8Values;
    public readonly short[] Int16Values;
    public readonly int[] Int32Values;
    public readonly long[] Int64Values;

    public readonly byte[] UInt8Values;
    public readonly ushort[] UInt16Values;
    public readonly uint[] UInt32Values;
    public readonly ulong[] UInt64Values;

    public readonly bool[] BooleanValues;
    public readonly float[] SingleValues;
    public readonly double[] DoubleValues;
    public readonly CigGuid[] GuidValues;

    public readonly DataCoreStringId[] StringIdValues;
    public readonly DataCoreStringId[] LocaleValues;
    public readonly DataCoreStringId[] EnumValues;

    public readonly DataCorePointer[] StrongValues;
    public readonly DataCorePointer[] WeakValues;
    public readonly DataCoreReference[] ReferenceValues;

    public readonly DataCoreStringId2[] EnumOptions;
    public readonly FrozenSet<CigGuid> MainRecords;

    public readonly int[] Offsets;
    private readonly DataCorePropertyDefinition[][] Properties;
    private readonly FrozenDictionary<int, string> CachedStrings;
    private readonly FrozenDictionary<int, string> CachedStrings2;
    private readonly FrozenDictionary<CigGuid, DataCoreRecord> RecordMap;

    public int StructsHash => StructDefinitions.Sum(x => x.FirstAttributeIndex ^ x.AttributeCount);

    public int EnumsHash => EnumDefinitions.Sum(x => x.FirstValueIndex ^ x.ValueCount);
    
    public DataCoreDatabase(Stream fs)
    {
        using var reader = new BinaryReader(fs);
        _ = reader.ReadUInt32();
        var version = reader.ReadUInt32();
        if (version is < 5 or > 8 or 7)
            throw new Exception($"Unsupported file version: {version}");
        _ = reader.ReadUInt32();
        _ = reader.ReadUInt32();

        var structDefinitionCount = reader.ReadInt32();
        var propertyDefinitionCount = reader.ReadInt32();
        var enumDefinitionCount = reader.ReadInt32();
        var dataMappingCount = reader.ReadInt32();
        var recordDefinitionCount = reader.ReadInt32();
        var booleanValueCount = reader.ReadInt32();
        var int8ValueCount = reader.ReadInt32();
        var int16ValueCount = reader.ReadInt32();
        var int32ValueCount = reader.ReadInt32();
        var int64ValueCount = reader.ReadInt32();
        var uint8ValueCount = reader.ReadInt32();
        var uint16ValueCount = reader.ReadInt32();
        var uint32ValueCount = reader.ReadInt32();
        var uint64ValueCount = reader.ReadInt32();
        var singleValueCount = reader.ReadInt32();
        var doubleValueCount = reader.ReadInt32();
        var guidValueCount = reader.ReadInt32();
        var stringIdValueCount = reader.ReadInt32();
        var localeValueCount = reader.ReadInt32();
        var enumValueCount = reader.ReadInt32();
        var strongValueCount = reader.ReadInt32();
        var weakValueCount = reader.ReadInt32();
        var referenceValueCount = reader.ReadInt32();
        var enumOptionCount = reader.ReadInt32();
        var textLength = reader.ReadUInt32();
        var textLength2 = reader.ReadUInt32();

        StructDefinitions = reader.BaseStream.ReadArray<DataCoreStructDefinition>(structDefinitionCount);
        PropertyDefinitions = reader.BaseStream.ReadArray<DataCorePropertyDefinition>(propertyDefinitionCount);
        EnumDefinitions = reader.BaseStream.ReadArray<DataCoreEnumDefinition>(enumDefinitionCount);
        DataMappings = reader.BaseStream.ReadArray<DataCoreDataMapping>(dataMappingCount);
        if (version >= 8)
        {
            RecordDefinitions = reader.BaseStream.ReadArray<DataCoreRecord>(recordDefinitionCount);
        }
        else
        {
            var v6Records = reader.BaseStream.ReadArray<DataCoreRecordV6>(recordDefinitionCount);
            RecordDefinitions = Array.ConvertAll(v6Records, r => DataCoreRecord.FromV6(in r));
        }

        Int8Values = reader.BaseStream.ReadArray<sbyte>(int8ValueCount);
        Int16Values = reader.BaseStream.ReadArray<short>(int16ValueCount);
        Int32Values = reader.BaseStream.ReadArray<int>(int32ValueCount);
        Int64Values = reader.BaseStream.ReadArray<long>(int64ValueCount);

        UInt8Values = reader.BaseStream.ReadArray<byte>(uint8ValueCount);
        UInt16Values = reader.BaseStream.ReadArray<ushort>(uint16ValueCount);
        UInt32Values = reader.BaseStream.ReadArray<uint>(uint32ValueCount);
        UInt64Values = reader.BaseStream.ReadArray<ulong>(uint64ValueCount);

        BooleanValues = reader.BaseStream.ReadArray<bool>(booleanValueCount);
        SingleValues = reader.BaseStream.ReadArray<float>(singleValueCount);
        DoubleValues = reader.BaseStream.ReadArray<double>(doubleValueCount);
        GuidValues = reader.BaseStream.ReadArray<CigGuid>(guidValueCount);

        StringIdValues = reader.BaseStream.ReadArray<DataCoreStringId>(stringIdValueCount);
        LocaleValues = reader.BaseStream.ReadArray<DataCoreStringId>(localeValueCount);
        EnumValues = reader.BaseStream.ReadArray<DataCoreStringId>(enumValueCount);

        StrongValues = reader.BaseStream.ReadArray<DataCorePointer>(strongValueCount);
        WeakValues = reader.BaseStream.ReadArray<DataCorePointer>(weakValueCount);
        ReferenceValues = reader.BaseStream.ReadArray<DataCoreReference>(referenceValueCount);
        EnumOptions = reader.BaseStream.ReadArray<DataCoreStringId2>(enumOptionCount);

        var stringTable1 = reader.ReadBytes((int)textLength);
        var stringTable2 = reader.ReadBytes((int)textLength2);

        CachedStrings = ReadStringTable(stringTable1);
        if (version >= 6)
            CachedStrings2 = ReadStringTable(stringTable2);
        else
            CachedStrings2 = CachedStrings;

        var bytesRead = (int)fs.Position;

        Offsets = ReadOffsets(bytesRead);
        DataSectionOffset = bytesRead;
        Properties = ReadProperties();
        DataSection = reader.ReadBytes((int)(fs.Length - bytesRead));
        RecordMap = RecordDefinitions.ToFrozenDictionary(x => x.Id);
        MainRecords = ReadMainRecords();
#if DEBUG
        DebugGlobal.Database = this;
#endif
    }

    private FrozenSet<CigGuid> ReadMainRecords()
    {
        var mainRecords = new Dictionary<int, DataCoreRecord>();

        foreach (var record in RecordDefinitions)
            mainRecords[record.FileNameOffset.Id] = record;

        return mainRecords.Values.Select(x => x.Id).ToFrozenSet();
    }

    public SpanReader GetReader(int structIndex, int instanceIndex)
    {
        var structTypeOffset = Offsets[structIndex];
        var size = StructDefinitions[structIndex].StructSize;
        var offset = structTypeOffset + size * instanceIndex;
        return new SpanReader(DataSection, (int)offset - DataSectionOffset);
    }

    public string GetString(DataCoreStringId id) => CachedStrings[id.Id];

    public string GetString2(DataCoreStringId2 id) => CachedStrings2[id.Id];
    public DataCoreRecord GetRecord(CigGuid guid) => RecordMap[guid];
    public DataCorePropertyDefinition[] GetProperties(int structIndex) => Properties[structIndex];

    private static FrozenDictionary<int, string> ReadStringTable(ReadOnlySpan<byte> span)
    {
        var strings = new Dictionary<int, string>();
        var offset = 0;

        while (offset < span.Length)
        {
            var length = span[offset..].IndexOf((byte)0);
            var useful = span[offset..(offset + length)];
            var str = Encoding.ASCII.GetString(useful);
            strings[offset] = str;
            offset += length + 1;
        }

        return strings.ToFrozenDictionary();
    }

    private static FrozenDictionary<int, int> ReadStringTableLengths(ReadOnlySpan<byte> span)
    {
        var strings = new Dictionary<int, int>();
        var offset = 0;

        while (offset < span.Length)
        {
            var length = span[offset..].IndexOf((byte)0);
            strings[offset] = length;
            offset += length + 1;
        }

        return strings.ToFrozenDictionary();
    }

    private int[] ReadOffsets(int initialOffset)
    {
        var offsets = new int[StructDefinitions.Length];

        var offset = initialOffset;
        foreach (var mapping in DataMappings.AsSpan())
        {
            var size = StructDefinitions[mapping.StructIndex].StructSize;
            offsets[mapping.StructIndex] = offset;
            offset += (int)(size * mapping.StructCount);
        }

        return offsets;
    }

    private DataCorePropertyDefinition[][] ReadProperties()
    {
        var result = new DataCorePropertyDefinition[StructDefinitions.Length][];

        for (var i = 0; i < StructDefinitions.Length; i++)
        {
            result[i] = GetStructProperties(i);
        }

        return result;
    }

    private DataCorePropertyDefinition[] GetStructProperties(int index)
    {
        var @this = StructDefinitions[index];
        var structs = StructDefinitions.AsSpan();
        var properties = PropertyDefinitions.AsSpan();

        if (@this is { AttributeCount: 0, ParentTypeIndex: -1 }) return [];

        // Calculate total property count to avoid resizing
        int totalPropertyCount = @this.AttributeCount;
        var baseStruct = @this;
        while (baseStruct.ParentTypeIndex != -1)
        {
            baseStruct = structs[baseStruct.ParentTypeIndex];
            totalPropertyCount += baseStruct.AttributeCount;
        }

        // Pre-allocate array with exact size needed
        var result = new DataCorePropertyDefinition[totalPropertyCount];

        // Reset to start struct for actual property copying
        baseStruct = @this;
        var currentPosition = totalPropertyCount;

        // Copy properties in reverse order to avoid InsertRange
        do
        {
            int count = baseStruct.AttributeCount;
            currentPosition -= count;
            properties.Slice(baseStruct.FirstAttributeIndex, count)
                .CopyTo(result.AsSpan(currentPosition, count));

            if (baseStruct.ParentTypeIndex == -1) break;
            baseStruct = structs[baseStruct.ParentTypeIndex];
        } while (true);

        return result;
    }
}